import type { Dirent } from "node:fs";
import { promises as fs } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type {
	Api,
	AssistantMessageEventStream,
	Context,
	Model,
	Provider,
	SimpleStreamOptions,
} from "@earendil-works/pi-ai";

const CODEX_PROVIDER_ID = "openai-codex";
const PROVIDER_PREFIX = "codex-switch-";
const PROFILE_FILE_PREFIX = "auth.json.";
const REFRESH_SKEW_MS = 5 * 60 * 1000;
const TOKEN_REFRESH_URL = "https://auth.openai.com/oauth/token";

let temporaryFileCounter = 0;

export interface Environment {
	readonly HOME?: string;
}

export interface PiProfileAuth {
	readonly access: string;
	readonly refresh?: string;
	readonly accountId?: string;
	readonly expires?: number;
	readonly type?: string;
}

export interface PiProfile {
	readonly name: string;
	readonly path: string;
	readonly auth: PiProfileAuth;
}

interface PiProfileDocument {
	readonly root: Record<string, unknown>;
	readonly rawEntry: Record<string, unknown>;
	readonly auth: PiProfileAuth;
}

interface RefreshResponse {
	readonly access_token?: string;
	readonly refresh_token?: string;
	readonly id_token?: string;
	readonly expires_in?: number;
}

const refreshLocks = new Map<string, Promise<PiProfileAuth>>();

export function codexSwitchStateDir(
	environment: Environment = process.env,
): string {
	return join(environment.HOME || homedir(), ".local", "state", "codex-switch");
}

export function piProfilesDir(environment: Environment = process.env): string {
	return join(codexSwitchStateDir(environment), "profiles", "pi");
}

export function piProfilePath(
	name: string,
	environment: Environment = process.env,
): string {
	return join(piProfilesDir(environment), `${PROFILE_FILE_PREFIX}${name}`);
}

export function providerIdForProfile(name: string): string {
	return `${PROVIDER_PREFIX}${name}`;
}

export function isValidProfileName(name: string): boolean {
	return (
		name.length > 0 && !name.startsWith("bak-") && /^[A-Za-z0-9_-]+$/.test(name)
	);
}

export function profileNameFromPath(path: string): string | undefined {
	const filename = path.slice(path.lastIndexOf("/") + 1);
	if (!filename.startsWith(PROFILE_FILE_PREFIX)) return undefined;
	const name = filename.slice(PROFILE_FILE_PREFIX.length);
	return isValidProfileName(name) ? name : undefined;
}

export async function listPiProfileNames(directory: string): Promise<string[]> {
	let entries: Dirent[];
	try {
		entries = await fs.readdir(directory, { withFileTypes: true });
	} catch (error) {
		if (isMissingFileError(error)) return [];
		throw error;
	}

	return entries
		.filter((entry) => entry.isFile())
		.map((entry) => profileNameFromPath(entry.name))
		.filter((name): name is string => name !== undefined)
		.sort();
}

function isMissingFileError(error: unknown): boolean {
	return (
		typeof error === "object" &&
		error !== null &&
		"code" in error &&
		error.code === "ENOENT"
	);
}

async function readProfileDocument(
	path: string,
): Promise<PiProfileDocument | undefined> {
	let content: string;
	try {
		content = await fs.readFile(path, "utf8");
	} catch (error) {
		if (isMissingFileError(error)) return undefined;
		throw error;
	}

	let parsed: unknown;
	try {
		parsed = JSON.parse(content);
	} catch {
		return undefined;
	}
	if (!isRecord(parsed)) return undefined;

	const rawEntry = parsed[CODEX_PROVIDER_ID];
	if (!isRecord(rawEntry) || typeof rawEntry.access !== "string") {
		return undefined;
	}

	return {
		root: parsed,
		rawEntry,
		auth: normalizeProfileAuth(rawEntry),
	};
}

function normalizeProfileAuth(entry: Record<string, unknown>): PiProfileAuth {
	return {
		access: entry.access as string,
		...(typeof entry.refresh === "string" ? { refresh: entry.refresh } : {}),
		...(typeof entry.accountId === "string"
			? { accountId: entry.accountId }
			: {}),
		...(typeof entry.expires === "number" ? { expires: entry.expires } : {}),
		...(typeof entry.type === "string" ? { type: entry.type } : {}),
	};
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

export async function discoverPiProfiles(
	environment: Environment = process.env,
): Promise<PiProfile[]> {
	const directory = piProfilesDir(environment);
	const names = await listPiProfileNames(directory);
	const profiles: PiProfile[] = [];

	for (const name of names) {
		const path = join(directory, `${PROFILE_FILE_PREFIX}${name}`);
		const document = await readProfileDocument(path);
		if (!document) continue;
		profiles.push({ name, path, auth: document.auth });
	}

	return profiles;
}

export function decodeJwtPayload(
	token: string,
): Record<string, unknown> | undefined {
	const parts = token.split(".");
	if (parts.length < 2) return undefined;
	try {
		const normalized = parts[1].replace(/-/g, "+").replace(/_/g, "/");
		const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, "=");
		const payload = JSON.parse(Buffer.from(padded, "base64").toString("utf8"));
		return isRecord(payload) ? payload : undefined;
	} catch {
		return undefined;
	}
}

function tokenExpiry(auth: PiProfileAuth): number | undefined {
	if (typeof auth.expires === "number") return auth.expires;
	const exp = decodeJwtPayload(auth.access)?.exp;
	return typeof exp === "number" ? exp * 1000 : undefined;
}

export function needsRefresh(auth: PiProfileAuth, now = Date.now()): boolean {
	const expires = tokenExpiry(auth);
	return expires !== undefined && expires <= now + REFRESH_SKEW_MS;
}

function tokenClientId(token: string): string | undefined {
	const payload = decodeJwtPayload(token);
	if (!payload) return undefined;
	if (typeof payload.client_id === "string" && payload.client_id.length > 0) {
		return payload.client_id;
	}

	const audience = payload.aud;
	const values = Array.isArray(audience)
		? audience
		: typeof audience === "string"
			? [audience]
			: [];
	return values.find(
		(value): value is string =>
			typeof value === "string" && value.startsWith("app_"),
	);
}

async function refreshProfileAuth(
	path: string,
	expected: PiProfileAuth,
	signal: AbortSignal,
): Promise<PiProfileAuth> {
	if (!expected.refresh) {
		throw new Error(`PI profile has no refresh token: ${path}`);
	}
	const clientId = tokenClientId(expected.access);
	if (!clientId) {
		throw new Error(`PI profile access token has no OAuth client id: ${path}`);
	}

	const response = await fetch(TOKEN_REFRESH_URL, {
		method: "POST",
		headers: { "content-type": "application/json" },
		body: JSON.stringify({
			client_id: clientId,
			grant_type: "refresh_token",
			refresh_token: expected.refresh,
		}),
		signal,
	});
	if (!response.ok) {
		throw new Error(`PI profile token refresh failed (${response.status})`);
	}

	const refreshed = (await response.json()) as RefreshResponse;
	if (!refreshed.access_token) {
		throw new Error("PI profile token refresh returned no access token");
	}
	const expiry = decodeJwtPayload(refreshed.access_token)?.exp;
	const expires =
		typeof expiry === "number"
			? expiry * 1000
			: typeof refreshed.expires_in === "number"
				? Date.now() + refreshed.expires_in * 1000
				: undefined;

	return {
		...expected,
		access: refreshed.access_token,
		...(refreshed.refresh_token
			? { refresh: refreshed.refresh_token }
			: expected.refresh
				? { refresh: expected.refresh }
				: {}),
		...(expires === undefined ? {} : { expires }),
	};
}

function sameProfileAuth(left: PiProfileAuth, right: PiProfileAuth): boolean {
	return (
		left.access === right.access &&
		left.refresh === right.refresh &&
		left.accountId === right.accountId &&
		left.expires === right.expires
	);
}

async function writeProfileAuthIfUnchanged(
	path: string,
	expected: PiProfileAuth,
	updated: PiProfileAuth,
): Promise<PiProfileAuth> {
	const current = await readProfileDocument(path);
	if (!current) throw new Error(`PI profile disappeared: ${path}`);
	if (!sameProfileAuth(current.auth, expected)) return current.auth;

	const nextEntry: Record<string, unknown> = {
		...current.rawEntry,
		...updated,
		type: current.rawEntry.type ?? "oauth",
	};
	const nextRoot = { ...current.root, [CODEX_PROVIDER_ID]: nextEntry };
	const content = `${JSON.stringify(nextRoot, null, 2)}\n`;
	const temporary = `${path}.tmp.${process.pid}.${temporaryFileCounter++}`;
	const mode = (await fs.stat(path)).mode & 0o777;

	try {
		await fs.writeFile(temporary, content, { mode });
		await fs.chmod(temporary, mode);
		await fs.rename(temporary, path);
	} catch (error) {
		await fs.rm(temporary, { force: true });
		throw error;
	}

	return updated;
}

async function loadEffectiveProfileAuth(
	profile: PiProfile,
	signal: AbortSignal,
): Promise<PiProfileAuth> {
	signal.throwIfAborted();
	const current = await readProfileDocument(profile.path);
	if (!current)
		throw new Error(`PI profile is missing or invalid: ${profile.name}`);
	if (!needsRefresh(current.auth)) return current.auth;
	if (!current.auth.refresh) {
		throw new Error(`PI profile access token is expired: ${profile.name}`);
	}

	const pending = refreshLocks.get(profile.path);
	if (pending) return pending;

	const refresh = (async () => {
		const latest = await readProfileDocument(profile.path);
		if (!latest)
			throw new Error(`PI profile is missing or invalid: ${profile.name}`);
		if (!needsRefresh(latest.auth)) return latest.auth;
		const updated = await refreshProfileAuth(profile.path, latest.auth, signal);
		return writeProfileAuthIfUnchanged(profile.path, latest.auth, updated);
	})();
	refreshLocks.set(profile.path, refresh);
	try {
		return await refresh;
	} finally {
		if (refreshLocks.get(profile.path) === refresh) {
			refreshLocks.delete(profile.path);
		}
	}
}

function accountAuth(profile: PiProfile) {
	const source = `codex-switch profile ${profile.name}`;
	return {
		apiKey: {
			name: `Codex account (${profile.name})`,
			async check({ signal }: { signal: AbortSignal }) {
				signal.throwIfAborted();
				const current = await readProfileDocument(profile.path);
				return current?.auth.access
					? { source, type: "api_key" as const }
					: undefined;
			},
			async resolve({ signal }: { signal: AbortSignal }) {
				const auth = await loadEffectiveProfileAuth(profile, signal);
				return { auth: { apiKey: auth.access }, source };
			},
		},
	};
}

function originalModel(provider: Provider, model: Model<Api>): Model<Api> {
	return { ...model, provider: provider.id };
}

export function buildAccountProvider(
	provider: Provider,
	profile: PiProfile,
): Provider {
	const providerId = providerIdForProfile(profile.name);
	const models = provider.getModels().map((model) => ({
		...model,
		provider: providerId,
		name: `${profile.name} · ${model.name}`,
	}));
	const accountProvider: Record<string, unknown> = {
		id: providerId,
		name: `Codex (${profile.name})`,
		baseUrl: provider.baseUrl,
		headers: provider.headers,
		auth: accountAuth(profile),
		getModels: () => models,
		filterModels: (available: readonly Model<Api>[]) => available,
		stream: (
			model: Model<Api>,
			context: Context,
			options?: Record<string, unknown>,
		): AssistantMessageEventStream =>
			provider.stream(
				originalModel(provider, model),
				context,
				options as never,
			),
		streamSimple: (
			model: Model<Api>,
			context: Context,
			options?: SimpleStreamOptions,
		): AssistantMessageEventStream =>
			provider.streamSimple(originalModel(provider, model), context, options),
	};

	if (provider.fetchDeferred) {
		accountProvider.fetchDeferred = (
			model: Model<Api>,
			handle: unknown,
			options?: Record<string, unknown>,
		) =>
			provider.fetchDeferred?.(
				originalModel(provider, model),
				handle as never,
				options as never,
			);
	}
	if (provider.cancelDeferred) {
		accountProvider.cancelDeferred = (
			model: Model<Api>,
			handle: unknown,
			options?: Record<string, unknown>,
		) =>
			provider.cancelDeferred?.(
				originalModel(provider, model),
				handle as never,
				options as never,
			);
	}

	return accountProvider as unknown as Provider;
}

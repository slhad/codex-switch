import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
	buildAccountProvider,
	decodeJwtPayload,
	discoverPiProfiles,
	isValidProfileName,
	needsRefresh,
	piProfilePath,
	providerIdForProfile,
} from "./codex-switch-accounts-lib.ts";

function jwt(payload: Record<string, unknown>): string {
	const encoded = Buffer.from(JSON.stringify(payload)).toString("base64url");
	return `header.${encoded}.signature`;
}

async function profileHome(name: string): Promise<string> {
	const home = await mkdtemp(join(tmpdir(), `codex-switch-accounts-${name}-`));
	await mkdir(join(home, ".local", "state", "codex-switch", "profiles", "pi"), {
		recursive: true,
	});
	return home;
}

async function writeProfile(
	home: string,
	name: string,
	entry: Record<string, unknown>,
): Promise<string> {
	const path = piProfilePath(name, { HOME: home });
	await writeFile(
		path,
		`${JSON.stringify({ "openai-codex": entry }, null, 2)}\n`,
		{ mode: 0o600 },
	);
	return path;
}

test("validates and namespaces profile names", () => {
	assert.equal(isValidProfileName("work"), true);
	assert.equal(isValidProfileName("personal_2"), true);
	assert.equal(isValidProfileName("bak-old"), false);
	assert.equal(isValidProfileName("bad/name"), false);
	assert.equal(providerIdForProfile("work"), "codex-switch-work");
});

test("discovers only valid PI OAuth profiles", async () => {
	const home = await profileHome("discovery");
	try {
		await writeProfile(home, "work", {
			type: "oauth",
			access: "access-work",
			refresh: "refresh-work",
			accountId: "account-work",
		});
		await writeProfile(home, "bak-old", {
			access: "ignored",
		});
		await writeFile(piProfilePath("broken", { HOME: home }), "not-json\n");

		const profiles = await discoverPiProfiles({ HOME: home });
		assert.deepEqual(
			profiles.map((profile) => profile.name),
			["work"],
		);
		assert.equal(profiles[0]?.auth.accountId, "account-work");
	} finally {
		await rm(home, { recursive: true, force: true });
	}
});

test("derives JWT payload expiry and refresh threshold", () => {
	const token = jwt({ exp: 1000, email: "person@example.com" });
	assert.deepEqual(decodeJwtPayload(token), {
		exp: 1000,
		email: "person@example.com",
	});
	assert.equal(needsRefresh({ access: token }, 750_000), true);
	assert.equal(needsRefresh({ access: token }, 200_000), false);
	assert.equal(decodeJwtPayload("not-a-jwt"), undefined);
});

test("account provider exposes profile-labelled models and profile auth", async () => {
	const home = await profileHome("provider");
	try {
		const profilePath = await writeProfile(home, "work", {
			type: "oauth",
			access: "access-work",
			refresh: "refresh-work",
		});
		const originalModel = {
			id: "gpt-5.3-codex",
			name: "GPT-5.3 Codex",
			api: "openai-codex-responses",
			provider: "openai-codex",
		};
		let delegatedProvider: string | undefined;
		const original = {
			id: "openai-codex",
			name: "OpenAI Codex",
			auth: {},
			getModels: () => [originalModel],
			stream: (model: { provider: string }) => {
				delegatedProvider = model.provider;
				return "stream";
			},
			streamSimple: () => "simple-stream",
		};
		const provider = buildAccountProvider(original as never, {
			name: "work",
			path: profilePath,
			auth: { access: "access-work", refresh: "refresh-work" },
		});

		assert.equal(provider.id, "codex-switch-work");
		assert.equal(provider.name, "Codex (work)");
		assert.deepEqual(
			provider.getModels().map((model) => ({
				id: model.id,
				name: model.name,
				provider: model.provider,
			})),
			[
				{
					id: "gpt-5.3-codex",
					name: "work · GPT-5.3 Codex",
					provider: "codex-switch-work",
				},
			],
		);

		const auth = provider.auth as unknown as {
			apiKey: {
				check(input: { signal: AbortSignal }): Promise<unknown>;
				resolve(input: { signal: AbortSignal }): Promise<unknown>;
			};
		};
		assert.deepEqual(
			await auth.apiKey.check({ signal: new AbortController().signal }),
			{
				source: "codex-switch profile work",
				type: "api_key",
			},
		);
		assert.deepEqual(
			await auth.apiKey.resolve({ signal: new AbortController().signal }),
			{
				auth: { apiKey: "access-work" },
				source: "codex-switch profile work",
			},
		);

		provider.stream(provider.getModels()[0] as never, {} as never, {} as never);
		assert.equal(delegatedProvider, "openai-codex");
	} finally {
		await rm(home, { recursive: true, force: true });
	}
});

test("expired profiles without refresh tokens fail closed", async () => {
	const home = await profileHome("expired");
	try {
		const path = await writeProfile(home, "expired", {
			type: "oauth",
			access: jwt({ exp: 100 }),
			expires: 100_000,
		});
		const provider = buildAccountProvider(
			{
				id: "openai-codex",
				name: "OpenAI Codex",
				auth: {},
				getModels: () => [],
				stream: () => undefined,
				streamSimple: () => undefined,
			} as never,
			{
				name: "expired",
				path,
				auth: { access: "stale" },
			},
		);

		const auth = provider.auth as unknown as {
			apiKey: {
				resolve(input: { signal: AbortSignal }): Promise<unknown>;
			};
		};
		await assert.rejects(
			auth.apiKey.resolve({ signal: new AbortController().signal }),
			/expired/,
		);
	} finally {
		await rm(home, { recursive: true, force: true });
	}
});

import type { OAuthCredential, Provider } from "@earendil-works/pi-ai";
import {
	type ExtensionAPI,
	readStoredCredential,
} from "@earendil-works/pi-coding-agent";

const PROVIDER_ID = "openai-codex";
const STATUS_ID = "codex-switch-auth-hot-reload";
const ORIGINAL_PROVIDER = Symbol.for(
	"codex-switch.original-openai-codex-provider",
);

type WrappedProvider = Provider & {
	[ORIGINAL_PROVIDER]?: Provider;
};

function currentDiskCredential(): OAuthCredential | undefined {
	const credential = readStoredCredential(PROVIDER_ID);
	return credential?.type === "oauth" ? credential : undefined;
}

export default function codexSwitchAuthHotReload(pi: ExtensionAPI) {
	let enabled = true;
	let installed = false;

	const updateStatus = (ctx: {
		ui: { setStatus(id: string, text: string | undefined): void };
	}) => {
		ctx.ui.setStatus(STATUS_ID, enabled ? "auth↻" : undefined);
	};

	const installProviderWrapper = (ctx: {
		modelRegistry: { getProvider(id: string): Provider | undefined };
		ui: {
			notify(message: string, level: "info" | "warning" | "error"): void;
			setStatus(id: string, text: string | undefined): void;
		};
	}) => {
		const registered = ctx.modelRegistry.getProvider(PROVIDER_ID) as
			| WrappedProvider
			| undefined;
		const provider = registered?.[ORIGINAL_PROVIDER] ?? registered;
		const oauth = provider?.auth.oauth;

		if (!provider || !oauth) {
			ctx.ui.notify(
				"codex-switch: openai-codex OAuth provider is unavailable",
				"warning",
			);
			return;
		}

		const wrapped = Object.create(provider) as WrappedProvider;
		Object.defineProperty(wrapped, ORIGINAL_PROVIDER, {
			value: provider,
			enumerable: false,
		});
		Object.defineProperty(wrapped, "auth", {
			value: {
				...provider.auth,
				oauth: {
					...oauth,
					async refresh(credential: OAuthCredential, signal?: AbortSignal) {
						if (!enabled) return oauth.refresh(credential, signal);

						const diskCredential = currentDiskCredential() ?? credential;
						if (diskCredential.expires > Date.now() + 60_000) {
							return diskCredential;
						}
						return oauth.refresh(diskCredential, signal);
					},
					async toAuth(credential: OAuthCredential) {
						const effective = enabled
							? (currentDiskCredential() ?? credential)
							: credential;
						return oauth.toAuth(effective);
					},
				},
			},
			enumerable: true,
		});

		pi.registerProvider(wrapped);
		installed = true;
		updateStatus(ctx);
	};

	pi.on("session_start", (_event, ctx) => {
		installProviderWrapper(ctx);
	});

	pi.on("session_shutdown", (_event, ctx) => {
		ctx.ui.setStatus(STATUS_ID, undefined);
	});

	pi.registerCommand("codex-switch-auth-reload", {
		description: "Enable, disable, or inspect Codex OAuth hot-reload",
		handler: async (args, ctx) => {
			const action = args.trim().toLowerCase() || "status";
			if (action === "on" || action === "enable" || action === "enabled") {
				enabled = true;
			} else if (
				action === "off" ||
				action === "disable" ||
				action === "disabled"
			) {
				enabled = false;
			} else if (action !== "status") {
				ctx.ui.notify(
					"Usage: /codex-switch-auth-reload [on|off|status]",
					"warning",
				);
				return;
			}

			updateStatus(ctx);
			ctx.ui.notify(
				`codex-switch OAuth hot-reload: ${
					enabled ? "enabled" : "disabled"
				}${installed ? "" : " (provider wrapper unavailable)"}`,
				enabled ? "info" : "warning",
			);
		},
	});
}

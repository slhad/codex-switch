import { builtinProviders } from "@earendil-works/pi-ai/providers/all";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

import {
	buildAccountProvider,
	discoverPiProfiles,
	providerIdForProfile,
} from "./codex-switch-accounts-lib.ts";

const CODEX_PROVIDER_ID = "openai-codex";

export default async function codexSwitchAccounts(pi: ExtensionAPI) {
	const baseProvider = builtinProviders().find(
		(provider) => provider.id === CODEX_PROVIDER_ID,
	);
	if (!baseProvider) return;

	const profiles = await discoverPiProfiles().catch(() => []);
	const providers = profiles.map((profile) => ({
		id: providerIdForProfile(profile.name),
		provider: buildAccountProvider(baseProvider, profile),
	}));
	const installed = new Set<string>();

	pi.on("session_start", async (_event, ctx) => {
		for (const { id, provider } of providers) {
			pi.registerProvider(provider);
			installed.add(id);
		}
		await ctx.modelRegistry.refresh({ allowNetwork: false });
	});

	pi.on("session_shutdown", () => {
		for (const providerId of installed) pi.unregisterProvider(providerId);
		installed.clear();
	});
}

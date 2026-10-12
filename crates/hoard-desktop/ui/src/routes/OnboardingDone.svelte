<script lang="ts">
  import { push } from "svelte-spa-router";
  import { fly } from "svelte/transition";
  import {
    ArrowRight,
    ShieldCheck,
    Sparkles,
    HardDrive,
    DownloadCloud,
  } from "@lucide/svelte";
  import { _ } from "svelte-i18n";
  import Button from "../lib/components/Button.svelte";
  import WizardShell from "../lib/components/WizardShell.svelte";
  import { auth } from "../lib/stores/auth";
  import { cloud } from "../lib/stores/cloud";
  import { clearOnboarding } from "../lib/stores/onboarding";
  import { wizardPreview } from "../lib/stores/debug";
  import { prefs, updatePrefs } from "../lib/stores/prefs";
  import * as api from "../lib/api";

  // Default the onboarding choice to the safer "backup only" mode.
  let mode = $state<api.SyncMode>("backup_only");

  async function finish() {
    // A preview must not rewrite the real sync mode and autostart.
    if ($wizardPreview) {
      push("/account");
      return;
    }
    await clearOnboarding();
    try {
      const updated = await api.setSyncMode(mode);
      prefs.set(updated);
    } catch (e) {
      console.warn("setting sync mode on finish failed:", e);
    }
    // Turn on autostart by default once setup completes, a save-sync tray
    // app is only useful if it's running. The OS-level enable can fail
    // (sandboxed/headless), so reflect what actually stuck back into prefs
    // and never let a failure block the user from reaching the dashboard.
    try {
      const enabled = await api.setAutostart(true);
      await updatePrefs({ autostart: enabled });
    } catch (e) {
      console.warn("enabling autostart on finish failed:", e);
    }
    push("/dashboard");
  }
</script>

<WizardShell step="done">
  <div in:fly={{ x: 24, duration: 220 }}>
    <div class="flex justify-center">
      <span
        class="flex h-14 w-14 items-center justify-center rounded-full border border-emerald-800/80 bg-black text-emerald-500"
      >
        <ShieldCheck size={26} />
      </span>
    </div>

    <h1
      class="mt-5 text-center text-2xl font-semibold tracking-tight text-zinc-50"
    >
      {#if $auth.user}
        {$_("onboarding_done.title_named", { values: { username: $auth.user.username } })}
      {:else if $cloud.account?.display_name}
        {$_("onboarding_done.title_named", { values: { username: $cloud.account.display_name } })}
      {:else}
        {$_("onboarding_done.title_generic")}
      {/if}
    </h1>
    <p class="mt-2 text-center text-sm text-zinc-400">
      {$_("onboarding_done.subtitle")}
    </p>

    {#if $auth.user}
      <dl
        class="mt-6 space-y-2 rounded-lg border border-white/[0.08] bg-layer-2 p-4 text-sm"
      >
        <div class="flex items-center justify-between gap-3">
          <dt class="text-zinc-500">{$_("onboarding_done.account")}</dt>
          <dd class="text-zinc-200">{$auth.user.username}</dd>
        </div>
        <div class="flex items-center justify-between gap-3">
          <dt class="text-zinc-500">{$_("onboarding_done.server")}</dt>
          <dd class="break-all text-right font-mono text-xs text-zinc-300">
            {$auth.user.server_url}
          </dd>
        </div>
        {#if $auth.user.is_admin}
          <div class="flex items-center justify-between gap-3">
            <dt class="text-zinc-500">{$_("onboarding_done.role")}</dt>
            <dd
              class="inline-flex items-center gap-1 rounded-full bg-emerald-500/10 px-2 py-0.5 text-xs text-emerald-300"
            >
              <Sparkles size={12} /> {$_("onboarding_done.administrator")}
            </dd>
          </div>
        {/if}
      </dl>
    {/if}

    <div class="mt-6 space-y-2">
      <p class="text-xs font-semibold uppercase tracking-wider text-zinc-500">
        {$_("settings.section_mode")}
      </p>
      {#each [{ m: "backup_only", icon: HardDrive, title: $_("settings.mode_backup_title"), desc: $_("settings.mode_backup_desc") }, { m: "full_sync", icon: DownloadCloud, title: $_("settings.mode_sync_title"), desc: $_("settings.mode_sync_desc") }] as opt (opt.m)}
        <button
          type="button"
          onclick={() => (mode = opt.m as api.SyncMode)}
          class="anim-host flex w-full items-start gap-3 rounded-lg border bg-black p-3 text-left transition-colors {mode ===
          opt.m
            ? 'border-emerald-600'
            : 'border-zinc-800 hover:border-zinc-600'}"
        >
          <span
            class="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md border bg-black {mode ===
            opt.m
              ? 'border-emerald-800/80 text-emerald-500'
              : 'border-zinc-800 text-zinc-400'}"
          >
            <opt.icon size={16} data-anim="pop" />
          </span>
          <div class="min-w-0 flex-1">
            <span class="text-sm font-medium text-zinc-100">{opt.title}</span>
            <p class="mt-0.5 text-xs text-zinc-400">{opt.desc}</p>
          </div>
        </button>
      {/each}
    </div>

    <div class="mt-7 flex justify-center">
      <Button variant="outline" size="lg" onclick={finish}>
        {$_("onboarding_done.continue")}
        <ArrowRight size={16} data-anim="nudge" />
      </Button>
    </div>
  </div>
</WizardShell>

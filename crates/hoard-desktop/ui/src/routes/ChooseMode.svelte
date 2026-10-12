<script lang="ts">
  import { push } from "svelte-spa-router";
  import { fly } from "svelte/transition";
  import { Server, Cloud } from "@lucide/svelte";
  import { _ } from "svelte-i18n";
  import WizardShell from "../lib/components/WizardShell.svelte";
  import { saveStep } from "../lib/stores/onboarding";

  async function chooseSelfHost() {
    await saveStep("server");
    push("/onboarding/server");
  }

  async function chooseCloud() {
    // Cloud now routes through the terms step, which owns the actual sign-in.
    await saveStep("terms");
    push("/onboarding/terms");
  }

  function back() {
    push("/onboarding/language");
  }
</script>

<WizardShell step="choose" onBack={back}>
  <div in:fly={{ x: 24, duration: 220 }}>
    <h1 class="text-xl font-semibold tracking-tight text-zinc-50">
      {$_("choose.title")}
    </h1>
    <p class="mt-2 text-sm text-zinc-400">
      {$_("choose.subtitle")}
    </p>

    <div class="mt-6 grid grid-cols-2 gap-3">
      <!-- Self-host — black -->
      <button
        type="button"
        onclick={chooseSelfHost}
        class="anim-host flex flex-col items-start gap-3 rounded-2xl border border-zinc-800 bg-black p-5 text-left transition-colors hover:border-zinc-600"
      >
        <span
          class="inline-flex h-10 w-10 items-center justify-center rounded-lg border border-zinc-800 bg-black text-zinc-200"
        >
          <Server size={20} data-anim="pop" />
        </span>
        <span class="text-sm font-semibold text-zinc-50">
          {$_("choose.selfhost_title")}
        </span>
        <span class="text-xs leading-relaxed text-zinc-400">
          {$_("choose.selfhost_desc")}
        </span>
      </button>

      <!-- Cloud, the accent -->
      <button
        type="button"
        onclick={chooseCloud}
        class="anim-host flex flex-col items-start gap-3 rounded-2xl border border-emerald-800/80 bg-black p-5 text-left transition-colors hover:border-emerald-600"
      >
        <span
          class="inline-flex h-10 w-10 items-center justify-center rounded-lg border border-emerald-800/80 bg-black text-emerald-500"
        >
          <Cloud size={20} data-anim="pop" />
        </span>
        <span class="text-sm font-semibold text-emerald-400">
          {$_("choose.cloud_title")}
        </span>
        <span class="text-xs leading-relaxed text-zinc-400">
          {$_("choose.cloud_desc")}
        </span>
      </button>
    </div>
  </div>
</WizardShell>

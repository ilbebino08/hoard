<script lang="ts">
  /**
   * Common chrome for every onboarding screen: vertically-centred card,
   * progress dots, optional back button, and a slot-rendered step name.
   *
   * Keeps each individual route (Welcome / ServerSetup / TokenSetup /
   * OnboardingDone) focused on its own form rather than re-implementing
   * navigation wiring.
   */
  import type { Snippet } from "svelte";
  import { ArrowLeft } from "@lucide/svelte";
  import { _ } from "svelte-i18n";
  import Button from "./Button.svelte";
  import StepDots from "./StepDots.svelte";
  import { auth } from "../stores/auth";
  import type { OnboardingStep } from "../stores/onboarding";

  type Props = {
    step: OnboardingStep;
    onBack?: () => void;
    children: Snippet;
  };

  let { step, onBack, children }: Props = $props();

  // The wizard forks at `choose` and both branches end on `done`. Until the
  // user picks a branch we show the shorter (Cloud) 4-dot track; the
  // self-hosted one expands to 5 once inside its server/token steps, or on
  // `done` with a self-hosted session. This kills the old bug where 5 dots
  // always showed and jumped from step 2 straight home.
  const CLOUD_ORDER: OnboardingStep[] = ["language", "choose", "terms", "done"];
  const SELFHOST_ORDER: OnboardingStep[] = [
    "language",
    "choose",
    "server",
    "token",
    "done",
  ];
  const order = $derived(
    step === "server" ||
      step === "token" ||
      (step === "done" && !!$auth.user && !$auth.user.is_cloud_server)
      ? SELFHOST_ORDER
      : CLOUD_ORDER,
  );
  const currentIndex = $derived(order.indexOf(step));
</script>

<div
  class="flex min-h-full flex-col items-center justify-center bg-black px-6 py-10"
>
  <div class="w-full max-w-md">
    <div class="mb-8 text-center text-xl font-semibold tracking-tight text-zinc-50">
      {$_("welcome.title")}
    </div>

    <!-- Card. No backdrop blur: on Windows it read as a grey haze (app.css). -->
    <div
      class="card-glass rounded-2xl border border-white/[0.08] bg-zinc-950/30 p-8 shadow-[0_1px_0_0_rgba(255,255,255,0.08)_inset,0_10px_40px_-12px_rgba(0,0,0,0.75)]"
    >
      {#if onBack}
        <Button variant="muted" class="mb-5" onclick={onBack}>
          <ArrowLeft size={14} data-anim="nudge-back" />
          {$_("common.back")}
        </Button>
      {/if}
      {@render children()}
    </div>

    <!-- Progress dots -->
    <div class="mt-6">
      <StepDots
        count={order.length}
        current={currentIndex}
        label={$_("wizard.step_aria", { values: { current: currentIndex + 1, total: order.length } })}
      />
    </div>
  </div>
</div>

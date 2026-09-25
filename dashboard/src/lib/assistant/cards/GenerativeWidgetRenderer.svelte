<script lang="ts">
  import type { TripPlan } from '$lib/api';
  import type { ActionCard, ActionResult, CalendarSlotCardData, JourneyOptionCardData } from '../types';
  import JourneyActionCard from './JourneyActionCard.svelte';
  import CalendarActionCard from './CalendarActionCard.svelte';
  import TelemetryPulseWidget from './TelemetryPulseWidget.svelte';
  import FeedDigestWidget from './FeedDigestWidget.svelte';
  import SpatialSummaryWidget from './SpatialSummaryWidget.svelte';
  import GenerativeCard from './GenerativeCard.svelte';
  import ActionChoiceWidget from './ActionChoiceWidget.svelte';

  let {
    card,
    onApplyJourney,
    onApplyCalendar,
    onPrompt,
  }: {
    card: ActionCard;
    onApplyJourney: (card: JourneyOptionCardData, plan: TripPlan | null) => Promise<ActionResult>;
    onApplyCalendar: (card: CalendarSlotCardData) => Promise<ActionResult>;
    onPrompt?: (prompt: string) => void;
  } = $props();
</script>

<div class="generative-widget-frame">
  {#if card.type === 'journey_option'}
    <JourneyActionCard card={card.data} onApply={onApplyJourney} />
  {:else if card.type === 'calendar_slot'}
    <CalendarActionCard card={card.data} onApply={onApplyCalendar} />
  {:else if card.type === 'telemetry_pulse'}
    <TelemetryPulseWidget card={card.data} />
  {:else if card.type === 'feed_digest'}
    <FeedDigestWidget card={card.data} />
  {:else if card.type === 'spatial_summary'}
    <SpatialSummaryWidget card={card.data} />
  {:else if card.type === 'generative_card'}
    <GenerativeCard card={card.data} onActionPrompt={onPrompt} />
  {:else if card.type === 'action_choice'}
    <ActionChoiceWidget card={card.data} onSelect={onPrompt} />
  {/if}
</div>

<style>
  .generative-widget-frame {
    display: contents;
  }
</style>

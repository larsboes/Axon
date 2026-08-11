/**
 * Lovelace card DSL.
 *
 * Every function here returns a plain JSON object in the shape Home Assistant's frontend
 * expects. Nothing in this file knows an entity ID, a room, or a house: callers pass those in.
 *
 * Division of labour, and it is deliberate: Bubble owns navigation and pop-ups, Mushroom owns
 * the content inside them, native cards own everything with a first-class implementation
 * already (tile, gauge, camera, energy). Two card sets that both draw buttons is how a
 * dashboard stops looking like one thing.
 *
 * Custom card names verified against the bundles actually installed on the instance:
 * bubble-card v3.2.5 registers one element and accepts card_type of button, pop-up,
 * separator, horizontal-buttons-stack, cover, climate, media-player, select, empty-column,
 * calendar. mushroom v5.2.2 registers one element per card.
 */

export type Card = Record<string, unknown>;
export type Condition = Record<string, unknown>;

/* ── native ─────────────────────────────────────────────────────────────────── */

/** A section heading. Replaces the markdown cards that were a third of the old dashboard. */
export const heading = (
  text: string,
  opts: { icon?: string; level?: "h1" | "h2" | "h3"; badges?: Card[]; tapNavigate?: string } = {},
): Card => ({
  type: "heading",
  heading: text,
  heading_style: opts.level === "h1" ? "title" : "subtitle",
  ...(opts.icon ? { icon: opts.icon } : {}),
  ...(opts.badges?.length ? { badges: opts.badges } : {}),
  ...(opts.tapNavigate
    ? { tap_action: { action: "navigate", navigation_path: opts.tapNavigate } }
    : {}),
});

/** A heading badge: small, background-less, for a number that needs no card of its own. */
export const headingBadge = (entity: string, opts: { icon?: string } = {}): Card => ({
  type: "entity",
  entity,
  ...(opts.icon ? { icon: opts.icon } : {}),
});

export const tile = (
  entity: string,
  opts: {
    name?: string;
    icon?: string;
    features?: Card[];
    featurePosition?: "bottom" | "inline";
    vertical?: boolean;
    hideState?: boolean;
    stateContent?: string | string[];
    tapAction?: Card;
    iconTapAction?: Card;
    colour?: string;
  } = {},
): Card => ({
  type: "tile",
  entity,
  ...(opts.name ? { name: opts.name } : {}),
  ...(opts.icon ? { icon: opts.icon } : {}),
  ...(opts.colour ? { color: opts.colour } : {}),
  ...(opts.vertical ? { vertical: true } : {}),
  ...(opts.hideState ? { hide_state: true } : {}),
  ...(opts.stateContent ? { state_content: opts.stateContent } : {}),
  ...(opts.features?.length
    ? { features: opts.features, feature_position: opts.featurePosition ?? "bottom" }
    : {}),
  ...(opts.tapAction ? { tap_action: opts.tapAction } : {}),
  ...(opts.iconTapAction ? { icon_tap_action: opts.iconTapAction } : {}),
});

/** Tile features. `light-brightness` is why a light tile beats a toggle row. */
export const feature = {
  brightness: (): Card => ({ type: "light-brightness" }),
  colourTemp: (): Card => ({ type: "light-color-temp" }),
  toggle: (): Card => ({ type: "toggle" }),
  vacuumCommands: (commands: string[]): Card => ({ type: "vacuum-commands", commands }),
  mowerCommands: (commands: string[]): Card => ({ type: "lawn-mower-commands", commands }),
  timerControl: (): Card => ({ type: "timer-actions", actions: ["cancel"] }),
};

export const gauge = (
  entity: string,
  opts: { name?: string; min?: number; max?: number; severity?: Card; needle?: boolean } = {},
): Card => ({
  type: "gauge",
  entity,
  ...(opts.name ? { name: opts.name } : {}),
  ...(opts.min !== undefined ? { min: opts.min } : {}),
  ...(opts.max !== undefined ? { max: opts.max } : {}),
  ...(opts.needle ? { needle: true } : {}),
  ...(opts.severity ? { severity: opts.severity } : {}),
});

/**
 * A camera, as a tile.
 *
 * Three card types were tried against this instance and all three rendered nothing:
 * picture-entity with camera_view "live" (what the old dashboard used, seven blank white
 * boxes), the same with "auto", and picture-glance with camera_image. The last one blanked
 * the whole view. The cameras themselves are healthy — all seven report `idle` with an
 * entity_picture, and the motion log shows Terrasse and Pool triggering minutes ago.
 *
 * So: a tile. Tapping it opens the more-info dialog with the live stream, which is what
 * anyone actually wants from a camera on a phone, and seven of them do not each try to hold
 * a stream open. Thumbnails are a follow-up, not a blocker.
 */
export const camera = (entity: string, name: string): Card => ({
  type: "tile",
  entity,
  name,
  icon: "mdi:cctv",
  tap_action: { action: "more-info" },
  icon_tap_action: { action: "more-info" },
});

export const conditional = (conditions: Condition[], card: Card): Card => ({
  type: "conditional",
  conditions,
  card,
});

export const markdown = (content: string): Card => ({ type: "markdown", content });

export const entities = (
  rows: (string | Card)[],
  opts: { title?: string; stateColor?: boolean } = {},
): Card => ({
  type: "entities",
  entities: rows,
  ...(opts.title ? { title: opts.title } : {}),
  ...(opts.stateColor === false ? {} : { state_color: true }),
});

export const grid = (cards: Card[], columns = 2, square = false): Card => ({
  type: "grid",
  columns,
  square,
  cards,
});

export const vstack = (cards: Card[]): Card => ({ type: "vertical-stack", cards });
export const hstack = (cards: Card[]): Card => ({ type: "horizontal-stack", cards });

export const energyDistribution = (): Card => ({ type: "energy-distribution", link_dashboard: true });

export const statisticsGraph = (
  entities_: string[],
  opts: { title?: string; days?: number; stat?: string; chart?: "bar" | "line" } = {},
): Card => ({
  type: "statistics-graph",
  entities: entities_,
  ...(opts.title ? { title: opts.title } : {}),
  days_to_show: opts.days ?? 30,
  stat_types: [opts.stat ?? "change"],
  chart_type: opts.chart ?? "bar",
});

export const historyGraph = (entities_: string[], hours = 24, title?: string): Card => ({
  type: "history-graph",
  entities: entities_,
  hours_to_show: hours,
  ...(title ? { title } : {}),
});

/* ── mushroom ───────────────────────────────────────────────────────────────── */

/**
 * The workhorse. A template card renders a Jinja string, which is how a machine state becomes
 * a German sentence without inventing a new template sensor for every card.
 */
export const mushTemplate = (opts: {
  primary: string;
  secondary?: string;
  icon?: string;
  iconColour?: string;
  entity?: string;
  tapAction?: Card;
  holdAction?: Card;
  multilineSecondary?: boolean;
  fill?: boolean;
}): Card => ({
  type: "custom:mushroom-template-card",
  primary: opts.primary,
  ...(opts.secondary ? { secondary: opts.secondary } : {}),
  ...(opts.icon ? { icon: opts.icon } : {}),
  ...(opts.iconColour ? { icon_color: opts.iconColour } : {}),
  ...(opts.entity ? { entity: opts.entity } : {}),
  ...(opts.tapAction ? { tap_action: opts.tapAction } : { tap_action: { action: "none" } }),
  ...(opts.holdAction ? { hold_action: opts.holdAction } : {}),
  ...(opts.multilineSecondary ? { multiline_secondary: true } : {}),
  ...(opts.fill ? { fill_container: true } : {}),
});

export const mushChips = (chips: Card[], alignment: "start" | "center" | "end" = "start"): Card => ({
  type: "custom:mushroom-chips-card",
  alignment,
  chips,
});

export const chip = {
  entity: (entity: string, opts: { icon?: string; useEntityPicture?: boolean; tapAction?: Card } = {}): Card => ({
    type: "entity",
    entity,
    ...(opts.icon ? { icon: opts.icon } : {}),
    ...(opts.tapAction ? { tap_action: opts.tapAction } : {}),
  }),
  template: (opts: { content?: string; icon?: string; iconColour?: string; tapAction?: Card }): Card => ({
    type: "template",
    ...(opts.content ? { content: opts.content } : {}),
    ...(opts.icon ? { icon: opts.icon } : {}),
    ...(opts.iconColour ? { icon_color: opts.iconColour } : {}),
    ...(opts.tapAction ? { tap_action: opts.tapAction } : { tap_action: { action: "none" } }),
  }),
  weather: (entity: string): Card => ({ type: "weather", entity, show_conditions: true, show_temperature: true }),
};

export const mushLight = (entity: string, opts: { name?: string; brightness?: boolean; colour?: boolean } = {}): Card => ({
  type: "custom:mushroom-light-card",
  entity,
  ...(opts.name ? { name: opts.name } : {}),
  show_brightness_control: opts.brightness ?? true,
  show_color_control: opts.colour ?? false,
  use_light_color: true,
  collapsible_controls: true,
});

/* ── bubble ─────────────────────────────────────────────────────────────────── */

/** The bottom navigation bar. One entry per view. */
export const bubbleNav = (
  links: { link: string; name: string; icon: string; pirSensor?: string }[],
  opts: { autoOrder?: boolean; highlightCurrent?: boolean } = {},
): Card => {
  const out: Card = {
    type: "custom:bubble-card",
    card_type: "horizontal-buttons-stack",
    auto_order: opts.autoOrder ?? false,
    highlight_current_view: opts.highlightCurrent ?? true,
  };
  // Bubble's navbar takes flat numbered keys rather than an array, so the loop writes
  // link_1/name_1/icon_1, link_2/… in order. Its own editor produces exactly this shape.
  links.forEach((l, i) => {
    out[`link_${i + 1}`] = l.link;
    out[`name_${i + 1}`] = l.name;
    out[`icon_${i + 1}`] = l.icon;
  });
  return out;
};

/** A pop-up sheet, addressed by the `#hash` a button navigates to. */
export const bubblePopUp = (opts: {
  hash: string;
  name: string;
  icon?: string;
  entity?: string;
  cards: Card[];
  width?: string;
}): Card => ({
  type: "custom:bubble-card",
  card_type: "pop-up",
  hash: opts.hash.startsWith("#") ? opts.hash : `#${opts.hash}`,
  name: opts.name,
  ...(opts.icon ? { icon: opts.icon } : {}),
  ...(opts.entity ? { entity: opts.entity } : {}),
  ...(opts.width ? { width_desktop: opts.width } : {}),
  bg_blur: 10,
  bg_opacity: 88,
  shadow_opacity: 0,
  is_sidebar_hidden: false,
});

/** The tile that opens a pop-up. */
export const bubbleButton = (opts: {
  entity?: string;
  name: string;
  icon?: string;
  navigate?: string;
  buttonType?: "switch" | "slider" | "state" | "name";
  showState?: boolean;
}): Card => ({
  type: "custom:bubble-card",
  card_type: "button",
  button_type: opts.buttonType ?? "switch",
  ...(opts.entity ? { entity: opts.entity } : {}),
  name: opts.name,
  ...(opts.icon ? { icon: opts.icon } : {}),
  show_state: opts.showState ?? false,
  ...(opts.navigate
    ? { tap_action: { action: "navigate", navigation_path: opts.navigate } }
    : {}),
});

export const bubbleSeparator = (name: string, icon?: string): Card => ({
  type: "custom:bubble-card",
  card_type: "separator",
  name,
  ...(icon ? { icon } : {}),
});

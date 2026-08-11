/**
 * View composition: turns a house model into the seven family views plus the technical one.
 *
 * One rule runs through all of it — every view answers exactly one question, and a card that
 * does not help answer that view's question belongs on another view or nowhere. That is what
 * collapsed eleven tabs into seven without losing a control.
 *
 * The other rule is negative: no card here explains the dashboard. The old one carried 48
 * markdown cards, a third of its total, telling the reader what the view they were looking at
 * was for. A heading does that in one line.
 */

import {
  camera,
  chip,
  conditional,
  entities,
  energyDistribution,
  feature,
  gauge,
  heading,
  historyGraph,
  mushChips,
  statisticsGraph,
  tile,
} from "./cards.ts";
import { type Section, type View, section, view } from "./views.ts";

/* eslint-disable @typescript-eslint/no-explicit-any */
type House = any;

/* ── shared ────────────────────────────────────────────────────────────────── */

/**
 * A klartext sentence as a card — a NATIVE tile whose entity is the sentence.
 *
 * This was a mushroom-template-card with the Jinja in `secondary`. On this instance
 * `custom:mushroom-template-card` does not render: a static `secondary` shows, a templated
 * one comes out empty, and inside a `conditional` the card disappears and takes the whole
 * view with it. Verified on 2026-08-11 with a probe dashboard, across a Home Assistant
 * restart and with card-mod both present and removed. Mushroom's CHIPS card is fine and is
 * still used for the room scenes.
 *
 * Nothing is lost by going native, because the sentence is already a sensor state: the tile
 * just displays it. Keep klartext sentences short — a tile's state line is one line and
 * ellipsises, which is why the wording in klartext.yaml is deliberately terse.
 */
const saysCard = (
  h: House,
  key: string,
  primary: string,
  icon: string,
  colour: string,
  tapPath?: string,
): Card =>
  tile(h.klartext[key], {
    name: primary,
    icon,
    colour,
    ...(tapPath ? { tapAction: { action: "navigate", navigation_path: tapPath } } : {}),
  });

/* ── navigation ─────────────────────────────────────────────────────────────── */

/** All seven, in the bottom bar, on both layouts. Lars chose seven over five knowing the
 *  labels would not fit at 390px; the icons carry it and the active one is highlighted. */
export const NAV = [
  { path: "start", name: "Start", icon: "mdi:home" },
  { path: "raeume", name: "Räume", icon: "mdi:sofa" },
  { path: "energie", name: "Strom", icon: "mdi:solar-power" },
  { path: "garten", name: "Garten", icon: "mdi:tree" },
  { path: "sicherheit", name: "Kameras", icon: "mdi:cctv" },
  { path: "automatik", name: "Automatik", icon: "mdi:robot" },
  { path: "system", name: "System", icon: "mdi:heart-pulse" },
];



/**
 * The navbar, as the first card of its own full-width section on every view.
 *
 * It was a view `header` first. That property took vertical space and rendered nothing:
 * Bubble's horizontal-buttons-stack is a card, and it pins itself to the bottom of the
 * viewport from wherever it is placed. Its own editor emits it as an ordinary card too.
 */
// column_span must never exceed the view's own max_columns. A section asking for four
// columns on a two-column phone view collapsed the whole grid and rendered a blank page.


/* ── views ──────────────────────────────────────────────────────────────────── */

const startView = (h: House, wall: boolean, slug: string): View =>
  view({
    title: "Start",
    path: "start",
    icon: "mdi:home",
    maxColumns: wall ? 4 : 2,
    sections: [
      // A plain, always-present status row.
      //
      // This was five `conditional` cards so the section could disappear when the house was
      // fine. On this instance a section of conditional cards renders the whole VIEW blank —
      // reproduced with native tiles inside them, so it is the conditional card and not
      // mushroom. Rather than ship a Start view that intermittently shows nothing, the
      // status is simply always there: four tiles, and the first one is green or red.
      //
      // What is lost is "silence when everything is fine". Worth getting back once the
      // conditional-card behaviour on 2026.6.3 is understood; not worth a blank dashboard.
      section(
        [
          heading("Status", { icon: "mdi:heart-pulse", level: "h1" }),
          tile(h.klartext.needsAttention, {
            name: "Haus",
            icon: "mdi:home-heart",
            stateContent: "state",
          }),
          tile(h.system.weakBatteries, { name: "Schwache Batterien", icon: "mdi:battery-alert" }),
          tile(h.system.lightsOffline, { name: "Lampen offline", icon: "mdi:lightbulb-alert" }),
          tile(h.system.updatesPending, {
            name: "Updates",
            icon: "mdi:package-up",
            tapAction: { action: "navigate", navigation_path: `/${slug}/system` },
          }),
        ],
        wall ? 4 : 2,
      ),
      section([
        heading("Schnellzugriff", { icon: "mdi:gesture-tap-button" }),
        ...h.ui.favourites.map((f: any) =>
          tile(f.entity, { name: f.name, icon: f.icon, vertical: true, hideState: false }),
        ),
        tile("light.wohnzimmer", {
          name: "Alle Lichter aus",
          icon: "mdi:lightbulb-off",
          hideState: true,
          tapAction: { action: "perform-action", perform_action: "light.turn_off", target: { entity_id: "all" } },
          iconTapAction: { action: "perform-action", perform_action: "light.turn_off", target: { entity_id: "all" } },
        }),
      ]),
      section([
        heading("Gerade im Haus", { icon: "mdi:eye" }),
        saysCard(h, "power", "Strom", "mdi:transmission-tower", "amber", `/${slug}/energie`),
        saysCard(h, "ev", "Auto", "mdi:car-electric", "blue"),
        saysCard(h, "mower", "Mähroboter", "mdi:robot-mower", "green", `/${slug}/garten`),
        saysCard(h, "pool", "Pool", "mdi:pool", "cyan", `/${slug}/garten`),
      ]),
    ],
  });

const roomsView = (h: House, wall: boolean, slug: string): View =>
  view({
    title: "Räume",
    path: "raeume",
    icon: "mdi:sofa",
    maxColumns: wall ? 4 : 2,
    sections: [...h.rooms.filter((r: any) => r.indoor).map((r: any) =>
      section([
        heading(r.name, {
          icon: r.icon,
          badges: r.lights.length > 1 ? [{ type: "entity", entity: r.lights[0].entity }] : undefined,
        }),
        ...r.lights.map((l: any) =>
          tile(l.entity, {
            name: l.name,
            // A light tile with a brightness slider replaces a toggle row and a separate
            // "dim" scene. Switch-backed garden lights get no slider because they have none.
            features: l.entity.startsWith("light.") ? [feature.brightness()] : undefined,
          }),
        ),
        ...(r.scenes.length
          ? [
              mushChips(
                r.scenes.map((s: any) =>
                  chip.template({
                    content: s.name,
                    icon: s.icon,
                    tapAction: { action: "perform-action", perform_action: "scene.turn_on", target: { entity_id: s.entity } },
                  }),
                ),
              ),
            ]
          : []),
      ]),
    )],
  });

const energyView = (h: House, wall: boolean, slug: string): View =>
  view({
    title: "Strom",
    path: "energie",
    icon: "mdi:solar-power",
    maxColumns: wall ? 4 : 2,
    sections: [
      section([
        heading("Gerade", { icon: "mdi:flash" }),
        saysCard(h, "power", "Strom", "mdi:transmission-tower", "amber"),
        tile(h.energy.solarPower, { name: "Solarleistung", icon: "mdi:solar-power" }),
        tile(h.energy.housePower, { name: "Hausverbrauch", icon: "mdi:home-lightning-bolt" }),
        tile(h.energy.batteryLevel, {
          name: "Hausakku",
          icon: "mdi:home-battery",
          stateContent: ["state", h.energy.batteryRuntime],
        }),
      ]),
      section([
        heading("Heute", { icon: "mdi:calendar-today" }),
        tile(h.energy.today.pv, { name: "Erzeugt", icon: "mdi:solar-power-variant" }),
        tile(h.energy.today.consumed, { name: "Verbraucht", icon: "mdi:home" }),
        tile(h.energy.today.exported, { name: "Eingespeist", icon: "mdi:transmission-tower-export" }),
        tile(h.energy.today.imported, { name: "Zugekauft", icon: "mdi:transmission-tower-import" }),
        tile(h.energy.today.batteryCharge, { name: "Akku geladen", icon: "mdi:battery-plus" }),
        tile(h.energy.today.batteryDischarge, { name: "Akku entladen", icon: "mdi:battery-minus" }),
        tile(h.energy.today.autarky, { name: "Autarkie", icon: "mdi:home-lightning-bolt" }),
        tile(h.energy.today.selfUse, { name: "Eigenverbrauch", icon: "mdi:solar-power" }),
      ]),
      section([
        heading("Auto laden", { icon: "mdi:ev-station" }),
        saysCard(h, "ev", "Ladestatus", "mdi:car-electric", "blue"),
        tile(h.energy.ev.enabled, { name: "Laden freigegeben" }),
        tile(h.energy.ev.session, { name: "Diese Ladung", icon: "mdi:battery-charging" }),
        tile(h.energy.ev.lifetime, { name: "Insgesamt geladen", icon: "mdi:counter" }),
        // The calendar drives the trip-charging automation, so it belongs beside the charger
        // rather than on the near-empty tab of its own it used to have.
        { type: "calendar", entities: [h.energy.ev.calendar], initial_view: "listWeek" },
      ]),
      section([
        heading("Vorhersage", { icon: "mdi:weather-partly-cloudy" }),
        tile(h.energy.forecast.remaining, { name: "Heute noch", icon: "mdi:weather-sunny" }),
        tile(h.energy.forecast.tomorrow, { name: "Morgen", icon: "mdi:weather-sunset-up" }),
        tile(h.energy.forecast.today, { name: "Heute gesamt", icon: "mdi:solar-power-variant" }),
        tile(h.energy.forecast.peak, { name: "Stärkste Stunde", icon: "mdi:chart-bell-curve" }),
        tile(h.energy.exportPower, { name: "Einspeisung jetzt", icon: "mdi:transmission-tower-export" }),
      ]),
      section(
        [
          heading("Verlauf", { icon: "mdi:chart-line" }),
          energyDistribution(),
          statisticsGraph([h.energy.today.pv, h.energy.today.consumed, h.energy.today.exported], {
            title: "Letzte 30 Tage",
            days: 30,
          }),
        ],
        wall ? 2 : 1,
      ),
    ],
  });

const gardenView = (h: House, wall: boolean, slug: string): View =>
  view({
    title: "Garten",
    path: "garten",
    icon: "mdi:tree",
    maxColumns: wall ? 4 : 2,
    sections: [
      section([
        heading("Pool", { icon: "mdi:pool" }),
        saysCard(h, "pool", "Wasser", "mdi:pool", "cyan"),
        tile(h.garden.pool.temperature, { name: "Temperatur", icon: "mdi:thermometer-water" }),
        tile(h.garden.pool.heater, { name: "Heizung", stateContent: ["state", h.garden.pool.heaterPower] }),
        tile(h.garden.pool.pump, { name: "Pumpe", stateContent: ["state", h.garden.pool.pumpPower] }),
        tile(h.garden.pool.light, { name: "Poollicht", icon: "mdi:lightbulb" }),
      ]),
      section([
        heading("Bewässerung", { icon: "mdi:sprinkler-variant" }),
        // One tile per zone carrying the manual switch, whether the schedule owns it, and
        // minutes left. The old dashboard put "Verbleibend 0 min" on its own row between
        // zones, so it was never obvious which zone the number belonged to.
        ...h.garden.irrigation.map((z: any) =>
          tile(z.manual, { name: z.name, stateContent: ["state", z.remaining] }),
        ),
        entities(
          h.garden.irrigation.map((z: any) => ({ entity: z.auto, name: `${z.name} – Automatik` })),
          { title: "Zeitplan aktiv" },
        ),
      ]),
      section([
        heading("Beete", { icon: "mdi:seed" }),
        ...h.garden.soil.map((s: any) => tile(s.entity, { name: s.name, icon: "mdi:water-percent" })),
      ]),
      section([
        heading("Mähroboter", { icon: "mdi:robot-mower" }),
        saysCard(h, "mower", "Navimow", "mdi:robot-mower", "green"),
        tile(h.garden.mower.entity, {
          name: "Mäher",
          features: [feature.mowerCommands(["start_mowing", "dock"])],
        }),
        tile(h.garden.mower.battery, { name: "Akku" }),
      ]),
      section([
        heading("Wetter im Garten", { icon: "mdi:weather-partly-rainy" }),
        tile(h.garden.weather.rain, { name: "Regnet es" }),
        tile(h.garden.weather.rainRate, { name: "Regenrate" }),
        tile(h.garden.weather.rainToday, { name: "Regen heute" }),
        tile(h.garden.mower.guardian.outdoorTemp, { name: "Außentemperatur" }),
      ]),
      section([
        heading("Licht draußen", { icon: "mdi:outdoor-lamp" }),
        ...h.rooms
          .filter((r: any) => !r.indoor)
          .flatMap((r: any) =>
            r.lights.map((l: any) =>
              tile(l.entity, {
                name: l.name,
                features: l.entity.startsWith("light.") ? [feature.brightness()] : undefined,
              }),
            ),
          ),
      ]),
    ],
  });

const securityView = (h: House, wall: boolean, slug: string): View =>
  view({
    title: "Kameras",
    path: "sicherheit",
    icon: "mdi:cctv",
    maxColumns: wall ? 4 : 2,
    sections: [
      section(
        [
          heading("Kameras", { icon: "mdi:cctv" }),
          ...h.security.cameras.map((c: any) => camera(c.entity, c.name)),
        ],
        wall ? 2 : 1,
      ),
      section([
        heading("Klingel", { icon: "mdi:bell" }),
        ...h.security.doorbells.map((d: any) =>
          tile(d.event, { name: d.name, icon: "mdi:bell-ring" }),
        ),
      ]),
      section([
        heading("Bewegung", { icon: "mdi:motion-sensor" }),
        ...h.security.motion.map((m: any) => tile(m.entity, { name: m.name })),
      ]),
      section([
        heading("Zuletzt gesehen", { icon: "mdi:history" }),
        entities(
          h.security.cameraEvents.map((e: any) => ({ entity: e.entity, name: e.name })),
        ),
      ]),
      section([
        heading("Innen gemessen", { icon: "mdi:thermometer" }),
        entities(h.security.indoorSensors.map((e: any) => ({ entity: e.entity, name: e.name }))),
      ]),
      section([
        heading("Kamera-Akkus", { icon: "mdi:battery" }),
        entities(
          h.security.cameras.map((c: any) => ({ entity: c.battery, name: c.name })),
        ),
      ]),
    ],
  });

const automationView = (h: House, wall: boolean, slug: string): View =>
  view({
    title: "Automatik",
    path: "automatik",
    icon: "mdi:robot",
    maxColumns: wall ? 4 : 2,
    sections: [
      section([
        heading("Hauptschalter", { icon: "mdi:shield-check", level: "h1" }),
        saysCard(h, "automation", "Automatik", "mdi:shield-check", "green"),
        tile(h.automation.master, { name: "Automatik scharf" }),
        tile(h.automation.overrideActive, { name: "Übersteuerung läuft" }),
        ...h.automation.domains.map((d: any) =>
          tile(d.status, {
            name: d.name,
            icon: d.icon,
            tapAction: {
              action: "perform-action",
              perform_action: "script.start_override",
              data: { timer_id: d.timer },
            },
          }),
        ),
      ]),
      ...h.automation.groups.map((g: any) =>
        section([
          heading(g.name, { icon: g.icon }),
          ...g.items.map((it: any) =>
            tile(it.entity, {
              name: it.text,
              features: [feature.toggle()],
              featurePosition: "inline",
              hideState: true,
            }),
          ),
        ]),
      ),
      section([
        heading("Staubsauger", { icon: "mdi:robot-vacuum" }),
        saysCard(h, "vacuum", "Staubsauger", "mdi:robot-vacuum", "purple"),
        tile(h.automation.vacuum.entity, {
          name: "Roborock",
          features: [feature.vacuumCommands(["start_pause", "return_home"])],
        }),
        tile(h.automation.vacuum.battery, { name: "Akku" }),
        tile(h.automation.vacuum.dnd, { name: "Bitte nicht stören" }),
        ...h.automation.vacuum.programmes.map((p: any) =>
          tile(p.entity, { name: p.name, icon: p.icon, hideState: true }),
        ),
      ]),
    ],
  });

const systemView = (h: House, wall: boolean, slug: string): View =>
  view({
    title: "System",
    path: "system",
    icon: "mdi:heart-pulse",
    maxColumns: wall ? 4 : 2,
    sections: [
      section([
        heading("Zustand", { icon: "mdi:heart-pulse", level: "h1" }),

        tile(h.system.updatesPending, { name: "Updates verfügbar", icon: "mdi:package-up" }),
        tile(h.system.weakBatteries, { name: "Schwache Batterien", icon: "mdi:battery-alert" }),
        tile(h.system.lightsOffline, { name: "Lampen nicht erreichbar", icon: "mdi:lightbulb-off" }),
      ]),
      section([
        heading("Updates", { icon: "mdi:package-up" }),
        entities(h.system.updates),
      ]),
      section([
        heading("Batterien", { icon: "mdi:battery" }),
        entities(h.system.batteries),
      ]),
      section([
        heading("Schwellen", { icon: "mdi:tune" }),
        entities(h.system.thresholds),
      ]),
    ],
  });

/* ── dashboards ─────────────────────────────────────────────────────────────── */

export const familyViews = (h: House, wall: boolean, slug: string): View[] => [
  startView(h, wall, slug),
  roomsView(h, wall, slug),
  energyView(h, wall, slug),
  gardenView(h, wall, slug),
  securityView(h, wall, slug),
  automationView(h, wall, slug),
  systemView(h, wall, slug),
];

export const technicalViews = (h: House): View[] => [
  view({
    title: "Netzwerk",
    path: "netzwerk",
    icon: "mdi:lan",
    maxColumns: 4,
    sections: [
      section([
        heading("FritzBox", { icon: "mdi:router-wireless" }),
        tile(h.network.fritz.status, { name: "Verbindung" }),
        tile(h.network.fritz.down, { name: "Download" }),
        tile(h.network.fritz.up, { name: "Upload" }),
        tile(h.network.fritz.cpuTemp, { name: "CPU-Temperatur" }),
        tile(h.network.fritz.uptime, { name: "Online seit" }),
      ]),
      section([
        heading("WLAN", { icon: "mdi:wifi" }),
        tile(h.network.fritz.wifi24, { name: "2.4 GHz" }),
        tile(h.network.fritz.wifi5, { name: "5 GHz" }),
        tile(h.network.fritz.guest, { name: "Gast-WLAN" }),
      ]),
      section([
        heading("Geräte", { icon: "mdi:devices" }),
        entities([
          { entity: h.network.devices.total, name: "Gesamt" },
          { entity: h.network.devices.wifi, name: "WLAN" },
          { entity: h.network.devices.guest, name: "Gast" },
          { entity: h.network.devices.lan, name: "LAN" },
        ]),
        tile(h.network.unknownCount, { name: "Unbekannte Geräte", icon: "mdi:help-network", colour: "orange" }),
        entities([{ entity: h.network.unknownList, name: "IP-Adressen" }]),
      ]),
      section([
        heading("Infrastruktur", { icon: "mdi:server-network" }),
        entities(h.network.infrastructure.map((i: any) => ({ entity: i.entity, name: i.name }))),
      ]),
      section([
        heading("Traffic", { icon: "mdi:swap-vertical" }),
        tile(h.network.fritz.sent, { name: "Gesendet" }),
        tile(h.network.fritz.received, { name: "Empfangen" }),
        historyGraph([h.network.fritz.down, h.network.fritz.up], 24, "Durchsatz 24 h"),
      ]),
    ],
  }),
  view({
    title: "Anlage",
    path: "anlage",
    icon: "mdi:cog",
    maxColumns: 4,
    sections: [
      section([
        heading("Wechselrichter", { icon: "mdi:solar-power" }),
        entities([
          h.energy.mode,
          h.energy.batteryStored,
          h.energy.batteryRuntime,
          "sensor.battery_charging_power",
          "sensor.battery_discharging_power",
        ]),
        gauge(h.energy.batteryLevel, { name: "Hausakku", min: 0, max: 100 }),
      ]),
      section([
        heading("EV-Schwellen", { icon: "mdi:tune" }),
        entities([...h.energy.tuning, h.energy.ev.request, h.energy.ev.smart]),
      ]),
      section([
        heading("Pool-Sollwerte", { icon: "mdi:pool" }),
        entities([
          h.garden.pool.setpointPh,
          h.garden.pool.setpointRedox,
          h.garden.pool.status,
          h.garden.pool.pumpTemp,
          h.garden.pool.phAlarm,
          h.garden.pool.redoxAlarm,
          h.garden.pool.tempAlarm,
        ]),
        gauge(h.garden.pool.ph, { name: "pH", min: 6, max: 8 }),
        gauge(h.garden.pool.redox, { name: "Redox mV", min: 400, max: 900 }),
      ]),
      section([
        heading("Interne Automationen", { icon: "mdi:cog-sync" }),
        entities(h.automation.internal),
      ]),
    ],
  }),
];

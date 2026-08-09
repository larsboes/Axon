export interface NavItem {
  href: string;
  label: string;
  icon: string;
  /**
   * The capability whose absence makes this destination pointless.
   *
   * Named for the published demo (#168), which runs on a subset of capabilities and has to
   * hide what it cannot show — a nav item leading to a page of error cards teaches a visitor
   * the software is broken rather than that the demo is partial. It is a real fact about the
   * route either way: /finance without the finance capability is an empty page on a real
   * machine too, and nothing but this line says so.
   *
   * Absent on /: Home draws from several capabilities and degrades to the ones present.
   */
  capability?: string;
}

/** Daily work stays visible. Machine administration sits one level deeper. */
export const PRIMARY_NAV: NavItem[] = [
  { href: "/", label: "Home", icon: "home" },
  { href: "/calendar", label: "Calendar", icon: "calendar", capability: "calendar" },
  { href: "/feed", label: "Feed", icon: "feed", capability: "comms" },
  { href: "/travel", label: "Travel", icon: "map-pin", capability: "transit" },
  { href: "/finance", label: "Finance", icon: "database", capability: "finance" },
];

/**
 * Projects and operations are real destinations, but not part of every daily pass.
 * Capability-owned sites are discovered on /projects rather than growing the main bar.
 */
export const UTILITY_NAV: NavItem[] = [
  { href: "/projects", label: "Projects", icon: "graduation" },
  { href: "/systems", label: "Systems", icon: "server", capability: "axon-status" },
  { href: "/capabilities", label: "Capabilities", icon: "boxes", capability: "axon-status" },
  { href: "/upstreams", label: "Upstreams", icon: "git-branch", capability: "axon-status" },
  { href: "/self", label: "Self-model", icon: "compass", capability: "axon-status" },
];

/** Drop the destinations a given set of missing capabilities makes pointless. */
export const withoutCapabilities = (items: NavItem[], missing: Set<string>): NavItem[] =>
  items.filter((item) => !item.capability || !missing.has(item.capability));

export const titleCase = (s: string): string => s.charAt(0).toUpperCase() + s.slice(1);

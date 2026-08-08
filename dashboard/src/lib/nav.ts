export interface NavItem {
  href: string;
  label: string;
  icon: string;
}

/** Daily work stays visible. Machine administration sits one level deeper. */
export const PRIMARY_NAV: NavItem[] = [
  { href: "/", label: "Home", icon: "home" },
  { href: "/calendar", label: "Calendar", icon: "calendar" },
  { href: "/feed", label: "Feed", icon: "feed" },
  { href: "/travel", label: "Travel", icon: "map-pin" },
];

/**
 * Projects and operations are real destinations, but not part of every daily pass.
 * Capability-owned sites are discovered on /projects rather than growing the main bar.
 */
export const UTILITY_NAV: NavItem[] = [
  { href: "/projects", label: "Projects", icon: "graduation" },
  { href: "/systems", label: "Systems", icon: "server" },
  { href: "/capabilities", label: "Capabilities", icon: "boxes" },
  { href: "/finance", label: "Finance", icon: "database" },
  { href: "/upstreams", label: "Upstreams", icon: "git-branch" },
  { href: "/self", label: "Self-model", icon: "compass" },
];

export const titleCase = (s: string): string => s.charAt(0).toUpperCase() + s.slice(1);

/**
 * Global omni-search modal state.
 * Accessible from anywhere in the shell via keyboard (Cmd+K, /) or the header button.
 */
class OmniStore {
  isOpen = $state(false);

  open(): void {
    this.isOpen = true;
  }

  close(): void {
    this.isOpen = false;
  }

  toggle(): void {
    this.isOpen = !this.isOpen;
  }
}

export const omniStore = new OmniStore();

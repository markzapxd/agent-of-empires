// Class recipe for a session row's active and selected chrome, resolved in one place so the rings don't fight by stylesheet order.

/** Purple active chrome for the open session; selected rows use a subtle purple fill instead of the legacy blue treatment. */
export function sessionRowChromeClass(isActive: boolean, isSelected: boolean): string {
  if (isActive) {
    return isSelected
      ? "ring-2 ring-inset ring-session-active bg-brand-500/20"
      : "ring-2 ring-inset ring-session-active bg-brand-500/10";
  }
  return isSelected
    ? "ring-1 ring-inset ring-brand-500/60 bg-brand-500/15 hover:bg-surface-700/40"
    : "hover:bg-surface-700/40";
}

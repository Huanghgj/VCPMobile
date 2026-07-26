export function advanceAuroraSequence(
  previousSequence: number,
  incomingSequence: number,
): number | null {
  if (!Number.isFinite(incomingSequence) || incomingSequence <= 0) {
    // Mixed-version compatibility: older backends did not send a sequence.
    return previousSequence;
  }

  return incomingSequence > previousSequence ? incomingSequence : null;
}

export function appendStableBlocksDelta<T>(
  currentBlocks: readonly T[],
  stableBlocksDelta: readonly T[],
): T[] {
  return [...currentBlocks, ...stableBlocksDelta];
}

export function playbackEndBody(
  handled: Set<string>,
  currentTrackId: string | undefined,
  trackId: string | undefined,
  playbackId: string | undefined,
  position?: number,
): { track_id: string; position?: number } | undefined {
  if (!trackId || trackId !== currentTrackId || !playbackId || handled.has(playbackId))
    return;
  handled.add(playbackId);
  if (handled.size > 64) handled.delete(handled.values().next().value!);
  return {
    track_id: trackId,
    ...(position !== undefined && Number.isFinite(position) && position >= 0
      ? { position }
      : {}),
  };
}

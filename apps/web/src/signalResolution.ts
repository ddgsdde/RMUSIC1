import type {AudioInfo, Endpoint, Track} from "@r-music/sdk";

type Path = NonNullable<Endpoint["playback"]>["signal_path"];

export function signalResolution(track: Track | null | undefined, info: AudioInfo | null, path: Path, contextRate = 0) {
  if (!track || !path?.sample_rate) return "";
  const source = info?.track_id === track.id ? info : track;
  const sourceRate = source.sample_rate ?? 0;
  if (path.sample_type === "dsd_native" || path.sample_type === "dsd_dop") {
    return sourceRate > 0 ? `DSD · ${sourceRate / 1e6} MHz` : "DSD";
  }
  const dsdToPcm = /^(dsf|dff|dsd)$/i.test(source.format ?? "");
  const rates = [path.sample_rate, path.mixer_sample_rate, contextRate, dsdToPcm ? 0 : sourceRate];
  const depths = [path.bit_depth, dsdToPcm ? 0 : source.bit_depth];
  for (const stage of path.processing ?? []) {
    const transport = /^(?:Remote FLAC transport|Core (?:FLAC transport|PCM conversion)): (\d+) Hz \/ (\d+) bit/.exec(stage);
    if (transport) {
      rates.push(Number(transport[1]));
      depths.push(Number(transport[2]));
    }
  }
  const known = (values: (number | undefined)[]) => values.filter((value): value is number => typeof value === "number" && Number.isFinite(value) && value > 0);
  return [known(depths).length ? `${Math.min(...known(depths))} bit` : "", `${Math.min(...known(rates)) / 1000} kHz`].filter(Boolean).join(" · ");
}

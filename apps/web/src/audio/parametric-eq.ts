import type {DspBand, EqFilter} from "@r-music/sdk";

export const filters: Record<EqFilter, {label: string; web: BiquadFilterType}> = {
  peak: {label: "峰值", web: "peaking"},
  low_shelf: {label: "低架", web: "lowshelf"},
  high_shelf: {label: "高架", web: "highshelf"},
  low_pass: {label: "低通", web: "lowpass"},
  high_pass: {label: "高通", web: "highpass"},
  band_pass: {label: "带通", web: "bandpass"},
  notch: {label: "陷波", web: "notch"},
  all_pass: {label: "全通", web: "allpass"},
};

export function equalizerNode(context: AudioContext, band: DspBand) {
  const kind = band.kind ?? "peak";
  const filter = context.createBiquadFilter();
  filter.type = filters[kind].web;
  filter.frequency.value = band.frequency;
  filter.Q.value = kind === "low_pass" || kind === "high_pass" ? 20 * Math.log10(band.q) : band.q;
  filter.gain.value = band.gain_db;
  return filter;
}

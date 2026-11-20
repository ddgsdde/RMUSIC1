import {expect,it} from "vitest";
import type {Endpoint,Track} from "@r-music/sdk";
import {signalResolution} from "./signalResolution";

it("reports the lowest known source, transport, mixer and output specifications", () => {
  const track = {id:"test",format:"flac",sample_rate:96000,bit_depth:16} as Track;
  const path = {sample_type:"integer",sample_rate:192000,bit_depth:32,mixer_sample_rate:88200,
    processing:["Core FLAC transport: 44100 Hz / 24 bit / 2 ch"]} as NonNullable<Endpoint["playback"]>["signal_path"];
  expect(signalResolution(track,null,path)).toBe("16 bit · 44.1 kHz");
  expect(signalResolution(track,null,undefined)).toBe("");
});

it("keeps native DSD separate from PCM and does not report one-bit PCM after DSD decoding", () => {
  const track = {id:"test",format:"dsf",sample_rate:2822400,bit_depth:1} as Track;
  const path = {sample_rate:176400,bit_depth:24,processing:[]} as NonNullable<Endpoint["playback"]>["signal_path"];
  expect(signalResolution(track,null,path)).toBe("24 bit · 176.4 kHz");
  expect(signalResolution(track,null,{...path,sample_type:"dsd_dop"} as NonNullable<Endpoint["playback"]>["signal_path"])).toBe("DSD · 2.8224 MHz");
});

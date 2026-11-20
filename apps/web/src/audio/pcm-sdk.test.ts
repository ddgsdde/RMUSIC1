import {afterEach,expect,it,vi} from "vitest";
import {RMusicClient} from "@r-music/sdk";
afterEach(()=>vi.restoreAllMocks());
it("builds continuous AAC URLs with fresh authority and the same output clock and prefix",async()=>{
  const client=new RMusicClient("https://core.example/external/music"),signal=new AbortController().signal;
  const ticket=vi.spyOn(client,"mediaUrl").mockResolvedValue("https://core.example/external/music/api/v1/media/qqmusic%3A123?ticket=limited-ticket");
  const result=new URL(await client.aacPcmUrl("qqmusic:123",192000,.5,signal));
  expect(result.pathname).toBe("/external/music/api/v1/transcode/qqmusic%3A123");expect(Object.fromEntries(result.searchParams)).toEqual({ticket:"limited-ticket",start:"0.5",pcm_rate:"192000",bitrate:"128"});expect(ticket).toHaveBeenCalledWith("qqmusic:123",signal);
});
it("builds PCM URLs through the existing session-bound ticket and preserves the reverse-proxy prefix",async()=>{
  const client=new RMusicClient("https://core.example/external/music"),signal=new AbortController().signal;
  const ticket=vi.spyOn(client,"mediaUrl").mockResolvedValue("https://core.example/external/music/api/v1/media/qqmusic%3A123?ticket=limited-ticket");
  expect(await client.pcmUrl("qqmusic:123",44100,.5,signal)).toBe("https://core.example/external/music/api/v1/pcm/qqmusic%3A123?ticket=limited-ticket&rate=44100&start=0.5");
  expect(ticket).toHaveBeenCalledWith("qqmusic:123",signal);
});
it("rejects unsupported clocks and invalid seeks before requesting a media ticket",async()=>{
  const client=new RMusicClient("https://core.example"),ticket=vi.spyOn(client,"mediaUrl");
  for(const [rate,start] of [[0,0],[192001,0],[48000.5,0],[48000,-1],[48000,NaN],[48000,Infinity]])await expect(client.pcmUrl("track",rate,start)).rejects.toThrow();
  expect(ticket).not.toHaveBeenCalled();
});
it("propagates an aborted ticket request instead of fetching a stale source",async()=>{
  const client=new RMusicClient("https://core.example"),signal=AbortSignal.abort();
  vi.spyOn(client,"mediaUrl").mockRejectedValue(new DOMException("cancelled","AbortError"));
  await expect(client.pcmUrl("track",48000,0,signal)).rejects.toMatchObject({name:"AbortError"});
});

it("native FLAC preserves prefix, scoped ticket, precise seek and validates caps before I/O",async()=>{
  const client=new RMusicClient("https://core.example/external/music"),signal=new AbortController().signal;
  const ticket=vi.spyOn(client,"mediaUrl").mockResolvedValue("https://core.example/external/music/api/v1/media/qqmusic%3A123?ticket=limited-ticket");
  const result=new URL(await client.nativeFlacUrl("qqmusic:123",{sample_rate:88200,bit_depth:24,channels:2,start:1.25},signal));
  expect(result.pathname).toBe("/external/music/api/v1/native-flac/qqmusic%3A123");
  expect(Object.fromEntries(result.searchParams)).toEqual({ticket:"limited-ticket",rate:"88200",bits:"24",channels:"2",start:"1.25"});expect(ticket).toHaveBeenCalledWith("qqmusic:123",signal);
  ticket.mockClear();await expect(client.nativeFlacUrl("x",{sample_rate:768001,bit_depth:24,channels:2})).rejects.toThrow();expect(ticket).not.toHaveBeenCalled();
});

it("mobile Opus preserves scoped authority and never changes global playback policy",async()=>{
 const client=new RMusicClient("https://core.example/prefix"),signal=new AbortController().signal;
 vi.spyOn(client,"mediaUrl").mockResolvedValue("https://core.example/prefix/api/v1/media/qq%3Aone?ticket=limited");
 const url=new URL(await client.mobileOpusUrl("qq:one",{bitrate:160,channels:2,start:12.5},signal));
 expect(url.pathname).toBe("/prefix/api/v1/mobile-opus/qq%3Aone");
 expect(Object.fromEntries(url.searchParams)).toEqual({ticket:"limited",accept_lossy:"true",bitrate:"160",channels:"2",start:"12.5"});
});
it("mobile Opus rejects invalid quality and positions before requesting authority",async()=>{
 const client=new RMusicClient("https://core.example"),ticket=vi.spyOn(client,"mediaUrl");
 for(const options of [{bitrate:47},{bitrate:321},{start:-1},{start:NaN}])await expect(client.mobileOpusUrl("track",options)).rejects.toThrow();
 expect(ticket).not.toHaveBeenCalled();
});

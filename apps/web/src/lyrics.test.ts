import { describe, expect, it } from "vitest";
import { activeLyric, parseLyrics, timedLyrics,lyricWordProgress } from "./lyrics";

it("word progress uses source durations and holds the end through silent gaps",()=>{
 const word={time:1.2,duration:.2,text:"甲"};expect(lyricWordProgress(word,1.1)).toBe(0);expect(lyricWordProgress(word,1.3)).toBeCloseTo(.5);expect(lyricWordProgress(word,2.8)).toBe(1);
 expect(lyricWordProgress({time:3,text:"乙"},3.01)).toBe(1);expect(lyricWordProgress({...word,duration:0},1.2)).toBe(1);
});

it("uses platform word durations only for matching line text and time",()=>{
 const document={text:"[00:01.00]Hello world\n[00:08.00]Other",word_timing:[{time:1,text:"Hello world",words:[{time:1.2,duration:.18,text:"Hello "},{time:3.4,duration:.7,text:"world"}]},{time:8,text:"Wrong version",words:[{time:8,duration:2,text:"Wrong version"}]}]};
 const lines=timedLyrics(document);
 expect(lines[0].words?.map(w=>w.duration)).toEqual([.18,.7]);expect(lines[0].words?.map(w=>w.text).join("")).toBe("Hello world");expect(lines[1].words).toBeUndefined();
 expect(timedLyrics({text:document.text})[0].words).toBeUndefined();
 document.word_timing[0].words[1].time=.5;expect(timedLyrics(document)[0].words).toBeUndefined();
});

it("accepts QQ colon fractions including instrumental notices",()=>{
  expect(parseLyrics("[00:00:00]纯音乐\n[00:12:25]正文\n[00:13:125]下一句").map(v=>[v.time,v.text])).toEqual([[0,"纯音乐"],[12.25,"正文"],[13.125,"下一句"]]);
});
import { validateTheme } from "./themes";
import { builtins } from "./studio/skins";
const themeFixture={schema_version:1,...builtins[0]};
describe("lyrics timing", () => {
  it("preserves repeated timestamps, translations and actual word timing", () => {
    const lines = parseLyrics(
      "[offset:200]\n[00:02.00]<00:02.00>晚风<00:03.00>掠过\n[00:02.00]Evening breeze\n[00:07.00][00:09.00]海面",
    );
    expect(lines).toHaveLength(3);
    expect(lines[0].time).toBe(2.2);
    expect(lines[0].translation).toBe("Evening breeze");
    expect(lines[0].words?.[1].time).toBe(3.2);
    expect(activeLyric(lines, 1)).toBe(-1);
    expect(activeLyric(lines, 7.3)).toBe(1);
  });
  it("does not invent timing for plain text", () => {
    expect(parseLyrics("第一行\n第二行").every((l) => l.time === -1)).toBe(
      true,
    );
  });
});
describe("theme import boundary", () => {
  it("rejects CSS injection and unsupported versions", () => {
    expect(() =>
      validateTheme({
        ...themeFixture,
        dark: { ...themeFixture.dark, bg: "url(https://evil.example)" },
      }),
    ).toThrow();
    expect(() => validateTheme({ ...themeFixture, schema_version: 2 })).toThrow();
  });
  it("strips executable or unknown properties", () => {
    const theme = validateTheme({
      ...themeFixture,
      script: "alert(1)",
      dark: { ...themeFixture.dark, extra: "bad" },
    });
    expect(theme).not.toHaveProperty("script");
    expect(theme.dark).not.toHaveProperty("extra");
  });
});

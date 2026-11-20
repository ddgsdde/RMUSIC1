export interface Word {
  time: number;
  text: string;
  duration?: number;
}
export interface LyricLine {
  time: number;
  text: string;
  translation?: string;
  words?: Word[];
}
export function lyricWordProgress(word:Word,position:number):number {
 if(!Number.isFinite(word.time)||!Number.isFinite(position)||position<word.time)return 0;
 return word.duration!==undefined&&word.duration>0&&Number.isFinite(word.duration)?Math.max(0,Math.min(1,(position-word.time)/word.duration)):1;
}
export function parseLyrics(text: string): LyricLine[] {
  const lines: LyricLine[] = [];
  let offset = 0;
  for (const raw of text.split(/\r?\n/)) {
    const off = raw.match(/\[offset:([+-]?\d+)\]/i);
    if (off) offset = Number(off[1]) / 1000;
    const stamps = [...raw.matchAll(/\[(\d+):(\d+(?:\.\d+|:\d{1,3})?)\]/g)];
    if (!stamps.length) continue;
    const body = raw.replace(/\[\d+:\d+(?:\.\d+|:\d{1,3})?\]/g, "").trim();
    const words = [...body.matchAll(/<(\d+):(\d+(?:\.\d+)?)>([^<]*)/g)].map(
      (m) => ({ time: Number(m[1]) * 60 + Number(m[2]), text: m[3] }),
    );
    for (const stamp of stamps) {
      const seconds = Number(stamp[2].replace(":", "."));
      if (seconds >= 60) continue;
      lines.push({
        time: Number(stamp[1]) * 60 + seconds,
        text: words.length ? words.map((w) => w.text).join("") : body,
        words: words.length ? words : undefined,
      });
    }
  }
  lines.sort((a, b) => a.time - b.time);
  const grouped: LyricLine[] = [];
  for (const line of lines) {
    line.time += offset;
    line.words?.forEach((w) => (w.time += offset));
    const last = grouped.at(-1);
    if (last && last.time === line.time) last.translation = line.text;
    else grouped.push(line);
  }
  return grouped.length
    ? grouped
    : text
        .split(/\r?\n/)
        .filter(Boolean)
        .map((line) => ({ time: -1, text: line }));
}
export function translatedLyrics(text:string,translation?:string):LyricLine[] {
 const original=parseLyrics(text);if(!translation)return original;
 const translated=parseLyrics(translation);
 if(original.every(line=>line.time<0)&&translated.every(line=>line.time<0))return original.map((line,index)=>({...line,translation:translated[index]?.text}));
 let index=0;
 return original.map(line=>{
  if(line.time<0)return line;
  while(index+1<translated.length&&translated[index+1].time<=line.time)index++;
  const candidate=[translated[index],translated[index+1]].filter(item=>item&&item.time>=0).sort((a,b)=>Math.abs(a.time-line.time)-Math.abs(b.time-line.time))[0];
  return candidate&&Math.abs(candidate.time-line.time)<=.08?{...line,translation:candidate.text}:line;
 });
}
export function timedLyrics(document: {text:string;word_timing?:{time:number;text:string;words:Word[]}[]}, translation?:string):LyricLine[] {
 const lines=translatedLyrics(document.text,translation),timings=document.word_timing;
 if(!Array.isArray(timings)||timings.length>2000)return lines;
 return lines.map(line=>{
  const timed=timings.find(v=>Number.isFinite(v.time)&&v.time>=0&&Math.abs(v.time-line.time)<=.08&&typeof v.text==="string"&&v.text.trim()===line.text);
  if(!timed||!Array.isArray(timed.words)||!timed.words.length||timed.words.length>500)return line;
  let previous=timed.time;
  for(const w of timed.words){if(!Number.isFinite(w.time)||w.time<previous||typeof w.duration!=="number"||!Number.isFinite(w.duration)||w.duration<0||w.time+w.duration>86400||typeof w.text!=="string"||!w.text)return line;previous=w.time;}
  if(timed.words.map(w=>w.text).join("")!==timed.text)return line;
  const words=timed.words.map(w=>({...w}));words[0].text=words[0].text.trimStart();words[words.length-1].text=words[words.length-1].text.trimEnd();
  return words.map(w=>w.text).join("")===line.text?{...line,words}:line;
 });
}
export function activeLyric(lines: LyricLine[], position: number) {
  let current = -1;
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].time >= 0 && lines[i].time <= position) current = i;
    else if (lines[i].time > position) break;
  }
  return current;
}
export function formatTime(value: number) {
  if (!Number.isFinite(value) || value < 0) return "0:00";
  return `${Math.floor(value / 60)}:${String(Math.floor(value % 60)).padStart(2, "0")}`;
}

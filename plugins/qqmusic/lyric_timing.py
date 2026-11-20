"""Decode already-decrypted QRC. Never manufacture word times from line length."""
from __future__ import annotations

import re
import xml.etree.ElementTree as ET


def is_qrc_xml(raw: str) -> bool:
    return bool(raw and re.match(r"\s*\ufeff?\s*(?:<\?xml[^>]*>\s*)?<QrcInfos\b", raw, re.I))


def line_lyrics(raw: str) -> str:
    if not raw or is_qrc_xml(raw):
        return ""
    return raw


def qrc_lines(raw: str) -> list[dict] | None:
    if not raw or len(raw.encode("utf-8")) > 1_048_576:
        return None
    if raw.lstrip().startswith("<"):
        if re.search(r"<!\s*(DOCTYPE|ENTITY)", raw, re.I):
            return None
        try:
            # QQ sometimes puts a literal ampersand in LyricContent.
            escaped = re.sub(r"&(?!amp;|lt;|gt;|apos;|quot;|#\d+;|#x[0-9a-fA-F]+;)", "&amp;", raw)
            root = ET.fromstring(escaped)
        except ET.ParseError:
            return None
        lyric = root.find(".//Lyric_1")
        if lyric is None:
            return None
        raw = lyric.get("LyricContent", "")
    stamps = list(re.finditer(r"\[(\d+),(\d+)\]", raw))
    if not stamps or len(stamps) > 2000:
        return None
    lines, total = [], 0
    for i, stamp in enumerate(stamps):
        start, span = map(int, stamp.groups())
        if start + span > 86_400_000:
            return None
        body = raw[stamp.end():stamps[i + 1].start() if i + 1 < len(stamps) else len(raw)]
        matches = list(re.finditer(r"(.*?)\((\d+),(\d+)\)", body, re.S))
        if not matches or len(matches) > 500 or body[matches[-1].end():].strip():
            return None
        words, previous = [], start
        for match in matches:
            text, time, duration = match.groups()
            time, duration = int(time), int(duration)
            if time < previous or time + duration > 86_400_000:
                return None
            previous = time
            if text:
                words.append({"time": time / 1000, "duration": duration / 1000, "text": text})
        if not words:
            continue
        total += len(words)
        if total > 20_000:
            return None
        lines.append({"time": start / 1000, "text": "".join(w["text"] for w in words), "words": words})
    return lines or None


def line_lrc(lines: list[dict]) -> str:
    return "\n".join(f"[{int(v['time'] // 60):02d}:{v['time'] % 60:06.3f}]{v['text']}" for v in lines)


def lyric_document(result) -> dict | None:
    lines = qrc_lines(result.lyric)
    if lines is None:
        return None
    def translated(raw):
        parsed = qrc_lines(raw)
        return line_lrc(parsed) if parsed else line_lyrics(raw)
    return {"format": "lrc", "text": line_lrc(lines), "word_timing": lines,
            "timing_source": "qqmusic_qrc", "translation": translated(result.trans),
            "romanization": translated(result.roma), "offset_ms": 0, "source": "qqmusic"}

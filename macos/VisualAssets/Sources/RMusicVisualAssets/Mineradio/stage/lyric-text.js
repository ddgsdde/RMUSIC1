// Mineradio d43de565acabfdc1a9c9820a27e81a98ccbebcef, GPL-3.0.
// R-Music port: selected visual declarations. Upstream DOM events, network and startup hooks excluded.

function isNoLyricText(text) {
  var compact = String(text || '').replace(/\s+/g, '').replace(/[，,。.!！?？、~～]/g, '');
  return !compact ||
    compact === '纯音乐请欣赏' ||
    compact === '暂无歌词' ||
    compact === '暂无歌词敬请期待' ||
    compact === '此歌曲为没有填词的纯音乐请您欣赏';
}

function normalizeLyricTranslationText(text) {
  text = normalizeStageLyricText(text);
  if (!text || isNoLyricText(text)) return '';
  return text;
}
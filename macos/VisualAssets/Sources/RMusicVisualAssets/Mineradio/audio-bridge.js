// Preserve all eight measured bands instead of expanding three aggregate values.
function rmusicSonicAudio(bands, energy, beat) {
  const b=Array.from({length:8},(_,i)=>Math.max(0,Math.min(1,Number(bands[i])||0)));
  return {sonicDetailed:true,subBass:b[0],bass:b[1],lowMid:b[2],mid:b[3],highMid:b[4],presence:b[5],brilliance:b[6],air:b[7],treble:(b[5]+b[6]+b[7])/3,energy,kickEnvelope:beat};
}

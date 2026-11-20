const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
test('the legacy stage uses source durations, keeps silence and lights ordinary LRC as a whole',()=>{
 const c=vm.createContext({fx:{},normalizeLyricFontKey:()=>'',lyricFontWeightValue:()=>750,lyricKaraokeWordRanges:()=>null});
 const source=fs.readFileSync(path.join(__dirname,'../Sources/RMusicVisualAssets/Mineradio/stage/14-stage-lyrics-rendering.js'),'utf8');
 vm.runInContext(source,c);c.lyricKaraokeWordRanges=()=>null;
 const ordinary={t:10,text:'甲乙',words:[],charCount:2};
 assert.equal(c.getLyricLineProgress(ordinary,{t:20},10.1),1);assert.equal(c.getLyricLineProgress(ordinary,{t:20},9.9),0);
 const timed={...ordinary,words:[{t:10.2,d:.2,c0:0,c1:1},{t:12,d:.8,c0:1,c1:2}]};
 assert(Math.abs(c.getLyricLineProgress(timed,null,10.3)-.25)<1e-6);assert.equal(c.getLyricLineProgress(timed,null,11.7),.5);
 timed.words[0].d=0;assert.equal(c.getLyricLineProgress(timed,null,10.2),.5);
});

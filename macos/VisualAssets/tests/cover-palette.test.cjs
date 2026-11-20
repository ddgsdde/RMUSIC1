const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const root=path.join(__dirname,'../Sources/RMusicVisualAssets/Mineradio');
function palette(rgb){
 const errors=[],c=vm.createContext({window:{},stageLyrics:{},fx:{lyricColorMode:'auto'},clampRange:(x,a,b)=>Math.max(a,Math.min(b,x)),console:{warn:e=>errors.push(String(e))}});
 const html=fs.readFileSync(path.join(root,'index.html'),'utf8');
 for(const name of ['color-utils.js','stage/07-lyrics-palette-text-utils.js'])if(html.includes('src="'+name+'"'))vm.runInContext(fs.readFileSync(path.join(root,name),'utf8'),c);
 c.setStageLyricPalette=p=>{c.applied=p;};
 const pixels=new Uint8ClampedArray(16*16*4);for(let i=0;i<pixels.length;i+=4)pixels.set([...rgb,255],i);
 c.updateLyricPaletteFromCover({width:16,height:16,getContext:()=>({getImageData:()=>({data:pixels})})});
 assert.equal(errors.length,0,errors.join('\n'));assert(c.stageLyrics.coverPalette,'cover extraction must produce a palette');
 return c.stageLyrics.coverPalette;
}
test('shipped stage extracts distinct red / blue cover colors and preserves neutral artwork',()=>{
 const red=palette([210,30,50]),blue=palette([30,70,210]),neutral=palette([128,128,128]);
 assert.notEqual(red.primary,blue.primary);assert.equal(red.rawPrimary,'rgb(210,30,50)');assert.equal(blue.rawPrimary,'rgb(30,70,210)');assert.equal(neutral.coverIsMonochrome,true);
});
test('Workshop receives cover regions and explicit custom color overrides',()=>{
 const c=vm.createContext({});vm.runInContext(fs.readFileSync(path.join(root,'three.r128.min.js'),'utf8'),c);vm.runInContext(fs.readFileSync(path.join(root,'workshop-properties.js'),'utf8'),c);
 const p=c.rmusicWorkshopProperties({}, {rawAreaPrimary:'#dd1133',rawAreaCool:'#2244cc',rawAreaWarm:'#ee4422'});
 assert.equal(p.mineradioCustomTheme.__primaryColor,'#dd1133');assert.equal(p.mineradioCustomTheme.uCoolCore,'#2244cc');
 const manual=c.rmusicWorkshopProperties({visualTintMode:'custom',visualTintColor:'#33aa55'},{});
 assert.equal(manual.mineradioCustomTheme.uWarmCore,'#33aa55');assert.equal(manual.mineradioCustomTheme.uCoolCore,'#33aa55');
});
test('shelf receives bounded images, cancels discarded image callbacks and releases cache',()=>{
 const images=[],c=vm.createContext({Image:class{constructor(){images.push(this);}},window:{},currentIdx:0});vm.runInContext(fs.readFileSync(path.join(root,'stage/shelf-adapter.js'),'utf8'),c);
 const src='data:image/jpeg;base64,AQ==',items=c.rmusicShelfQueue([{id:'a',index:7,cover:src},{id:'b',cover:'https://not-allowed.invalid/x.jpg'}]);
 assert.equal(items[0].cover,src);assert.equal(items[1].cover,'');assert.equal(c.rmusicShelfQueue(Array(100).fill({})).length,50);
 let calls=0;c.requestPlaylistCover(src,()=>calls++);const late=images[0].onload;c.prunePlaylistCovers([]);late();assert.equal(calls,0);assert.equal(Object.keys(c.playlistCoverCache).length,0);assert.equal(images[0].src,'');
 for(let i=0;i<20;i++)c.requestPlaylistCover(src+i,()=>{});assert.equal(Object.keys(c.playlistCoverCache).length,11);c.prunePlaylistCovers([]);
});
test('measured low / high bands remain distinct through the stage adapter',()=>{
 const c=vm.createContext({});vm.runInContext(fs.readFileSync(path.join(root,'audio-bridge.js'),'utf8'),c);
 const low=c.rmusicSonicAudio([1,0,0,0,0,0,0,0],.7,.5),high=c.rmusicSonicAudio([0,0,0,0,0,0,0,1],.2,0);
 assert.equal(low.subBass,1);assert.equal(low.air,0);assert.equal(high.air,1);assert.equal(high.subBass,0);assert.equal(high.kickEnvelope,0);
});
test('focused shelf cover and text stay inside the window with wide / narrow cameras',()=>{
 const c=vm.createContext({});vm.runInContext(fs.readFileSync(path.join(root,'three.r128.min.js'),'utf8'),c);vm.runInContext(fs.readFileSync(path.join(root,'shelf-viewport.js'),'utf8'),c);
 const T=c.THREE,fit=c.rmusicShelfViewportFit();
 for(const aspect of [1,1.58,2.4])for(const cameraX of [-.4,0,.4]){
  const camera=new T.PerspectiveCamera(45,aspect,.1,100);camera.position.set(cameraX,0,6);camera.lookAt(0,0,0);camera.updateMatrixWorld();
  const group=new T.Group(),mesh=new T.Mesh(new T.PlaneGeometry(2.5,1.1));group.add(mesh);mesh.position.set(3.18,0,.86);
  fit({mesh},camera);
  for(const x of [-1.25,1.25])for(const y of [-.55,.55]){const p=new T.Vector3(x,y,0).applyMatrix4(mesh.matrixWorld).project(camera);assert(p.x<=.941&&p.x>=.159,`${aspect}: ${p.x}`);}
 }
});
test('real shelf scroll remains visual-only, bounded and usable after repeated window changes',()=>{
 const ctx=new Proxy({measureText:t=>({width:String(t).length*16}),createLinearGradient:()=>({addColorStop(){}})}, {get:(o,k)=>k in o?o[k]:(()=>{})});
 const c=vm.createContext({fx:{},console,document:{body:{classList:{contains:()=>false}},createElement:()=>({getContext:()=>ctx})},innerWidth:1360,innerHeight:860,hasAnyPlatformLogin:()=>false,shouldUseSkullSafeShelfCamera:()=>false,shouldUseWallpaperSafeShelfCamera:()=>false,shelfPinnedOpen:true,uniforms:{uTime:{value:2}},bass:0,beatPulse:0,normalizeHexColor:(x,f)=>x||f,clampRange:(x,a,b)=>Math.max(a,Math.min(b,x)),hexToRgb:()=>({r:255,g:255,b:255}),renderer:{initTexture(){}},cancelAnimationFrame(){},requestAnimationFrame(){return 1;}});
 c.window=c;
 for(const file of ['three.r128.min.js','stage/defaults.js','shelf-viewport.js','stage/shelf-settings.js','stage/shelf-layout.js','stage/shelf-adapter.js','stage/shelf-manager.js'])vm.runInContext(fs.readFileSync(path.join(root,file),'utf8'),c);
 c.fx={...c.fxDefaults};c.scene=new c.THREE.Scene();c.currentIdx=0;c.playQueue=Array.from({length:50},(_,i)=>({id:String(i),name:'Track '+i,coreIndex:i}));
 const shelf=c.makeShelfManager();shelf.setMode('side');
 for(let i=0;i<30;i++)shelf.scrollBy(1);
 assert(shelf.getCards().length<=11);assert(shelf.getCards().some(card=>card.index===30));
 shelf.refreshCovers();assert(shelf.getCards().some(card=>card.index===30),'cover arrival must not reset the scrolled position');
 shelf.setMode('off');assert.equal(shelf.getCards().length,0);
});

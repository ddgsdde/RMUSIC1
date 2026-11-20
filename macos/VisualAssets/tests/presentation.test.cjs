const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const root=path.join(__dirname,'../Sources/RMusicVisualAssets/Mineradio');
function load(name,globals={}){const c=vm.createContext(globals);vm.runInContext(fs.readFileSync(path.join(root,name),'utf8'),c);return c;}
test('frame caps preserve their cadence with quantized 16 ms WebKit callbacks instead of dropping to 20 fps',()=>{
 const c=load('presentation.js');for(const fps of [24,30,60]){const pacer=c.rmusicFramePacer();let draws=0;for(let now=0;now<1000;now+=16)if(pacer.due(now,fps))draws++;assert(Math.abs(draws-fps)<=1,`${fps}: ${draws}`);pacer.reset();assert(pacer.due(10000,fps));}
});
test('audible clock compensates IPC delivery, keeps advancing without drawing, and freezes paused/stale samples',()=>{
 const c=load('presentation.js');let now=1000,wall=10000;const clock=c.rmusicPresentationClock(()=>now,()=>wall);
 clock.set({position:12,playing:true,sentAtMS:9950,duration:30});now+=100;assert(Math.abs(clock.read()-12.15)<1e-8);
 now+=500;assert(Math.abs(clock.read()-12.65)<1e-8);clock.set({position:13,playing:false});now+=1000;assert.equal(clock.read(),13);
 clock.set({position:4,playing:true});now+=10000;assert.equal(clock.read(),6);clock.set({position:1,playing:true});assert.equal(clock.read(),1);
});
test('feature batches are scheduled by audible position, never play future beats, reject stale sessions and stay bounded',()=>{
 const c=load('presentation.js'),timeline=c.rmusicFeatureTimeline(),frame=(position,playback_id='p')=>({position,playback_id,bands:Array(8).fill(100),energy:120,peak:130});
 timeline.push([frame(1),frame(1.05),frame(1.1),frame(1.15,'old')],'p');assert.equal(timeline.sample(.9),null);
 assert.equal(timeline.sample(1.06).position,1.05);assert.equal(timeline.sample(1.11).position,1.1);assert.equal(timeline.sample(1.5),null);
 for(let i=0;i<100;i++)timeline.push([frame(i+2)],'p');assert(timeline.size()<=48);timeline.clear();assert.equal(timeline.size(),0);
});
test('short transients between draws are consumed once in source order instead of skipping to the final quiet frame',()=>{
 const c=load('presentation.js'),timeline=c.rmusicFeatureTimeline(),frame=(position,low)=>({position,playback_id:'p',bands:Array(8).fill(low),energy:low,peak:low});
 timeline.push([frame(0,0),frame(.04,204),frame(.08,0),frame(.2,255)],'p');
 assert.deepEqual(Array.from(timeline.drain(.1),f=>f.position),[0,.04,.08]);
 assert.equal(timeline.sample(.1).position,.08);assert.equal(timeline.drain(.1).length,0);
 assert.equal(timeline.drain(.2)[0].position,.2);
 timeline.clear();timeline.push([frame(0,204)],'p');assert.equal(timeline.drain(0).length,1);
});
test('owned glyph disposal releases GPU accounting and canvas backing storage exactly once',()=>{
 const c=load('lyric-resources.js');const canvas={width:2048,height:512,getContext(){}};const events=new Map();const texture={image:canvas,userData:{__mineradioLyricOwned:true},generateMipmaps:true,addEventListener(n,fn){events.set(n,fn);},removeEventListener(n){events.delete(n);}};
 c.rmusicLyricResources.own(texture);c.rmusicLyricResources.own(texture);assert.equal(c.rmusicLyricResources.metrics().count,1);assert(c.rmusicLyricResources.metrics().bytes>8*1024*1024);
 events.get('dispose')();assert.equal(c.rmusicLyricResources.metrics().bytes,0);assert.equal(c.rmusicLyricResources.metrics().count,0);assert.equal(canvas.width,1);assert.equal(texture.image,null);
});
test('lyric preparation owns only one future mesh and cancels partial builds when hidden or replaced',()=>{
 const pending=new Map(),disposed=[],cancelled=[];let serial=0;
 const c=load('lyric-scheduler.js',{setTimeout(fn){const id=++serial;pending.set(id,fn);return id;},clearTimeout(id){pending.delete(id);},document:{hidden:false},visualHostVisible:true,console,window:{rmusicVisual:{redraw(){}}},renderer:{initTexture(){}},beginCooperativeLyricMeshBuild(payload){return {payload,steps:0};},stepCooperativeLyricMeshBuild(state){return ++state.steps===3;},finishCooperativeLyricMeshBuild(state){return {id:state.payload.key,traverse(){}};},cancelCooperativeLyricMeshBuild(state){cancelled.push(state.payload.key);},disposeLyricMesh(mesh){disposed.push(mesh.id);}});
 const scheduler=c.rmusicLyricScheduler();function drain(){for(let i=0;pending.size&&i<100;i++){const [id,fn]=pending.entries().next().value;pending.delete(id);fn();}assert.equal(pending.size,0);}
 scheduler.prepare({key:'a'});scheduler.prepare({key:'b'});drain();assert.deepEqual(cancelled,['a']);assert.equal(scheduler.take({key:'a'}),null);assert.equal(scheduler.take({key:'b'}).id,'b');
 scheduler.prepare({key:'c'});drain();scheduler.prepare({key:'d'});assert.deepEqual(disposed,['c']);c.document.hidden=true;drain();assert(cancelled.includes('d'));scheduler.clear();assert(!scheduler.pending());
});

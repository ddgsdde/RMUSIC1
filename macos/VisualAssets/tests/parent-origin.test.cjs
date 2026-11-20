const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const root=path.join(__dirname,'../Sources/RMusicVisualAssets/Mineradio');
function context(protocol,origin){const parent={},sandbox={parent,location:{protocol,origin}};vm.createContext(sandbox);vm.runInContext(fs.readFileSync(path.join(root,'parent-origin.js'),'utf8'),sandbox);return sandbox;}
test('WK local file frames accept both observed opaque/local origin serializations, only from their parent',()=>{
 const c=context('file:','file://');assert(c.rmusicTrustedParent({source:c.parent,origin:'null'}));assert(c.rmusicTrustedParent({source:c.parent,origin:'file://'}));assert(!c.rmusicTrustedParent({source:{},origin:'null'}));assert(!c.rmusicTrustedParent({source:c.parent,origin:'https://elsewhere.test'}));
});
test('HTTP frames retain exact origin validation',()=>{
 const c=context('http:','http://127.0.0.1:1234');assert(c.rmusicTrustedParent({source:c.parent,origin:c.location.origin}));for(const origin of ['null','file://','http://127.0.0.1:1235'])assert(!c.rmusicTrustedParent({source:c.parent,origin}));
});
test('the actual Workshop scheduler starts queued frames on local-file activity and stops on background',()=>{
 const c=context('file:','file://'),events={},timers=new Map(),frames=new Map();let id=0,draws=0;
 c.window={devicePixelRatio:2,requestAnimationFrame:cb=>{frames.set(++id,cb);return id;},cancelAnimationFrame:key=>frames.delete(key),addEventListener:(kind,cb)=>events[kind]=cb,dispatchEvent:()=>{}};
 c.setTimeout=cb=>{timers.set(++id,cb);return id;};c.clearTimeout=key=>timers.delete(key);c.Event=function(){};
 vm.runInContext(fs.readFileSync(path.join(root,'presentation.js'),'utf8'),c);
 vm.runInContext(fs.readFileSync(path.join(root,'workshop/scheduler.js'),'utf8'),c);
 c.window.requestAnimationFrame(()=>draws++);assert.equal(timers.size,0);
 events.message({source:{},origin:'null',data:{type:'rmusic-workshop-activity',active:true,fps:60}});assert.equal(timers.size,0);
 events.message({source:c.parent,origin:'null',data:{type:'rmusic-workshop-activity',active:true,fps:60,scale:1}});assert.equal(timers.size,0);assert.equal(frames.size,1);
 const frame=[...frames.values()][0];frames.clear();frame(100);assert.equal(draws,1);
 c.window.requestAnimationFrame(()=>draws++);events.message({source:c.parent,origin:'file://',data:{type:'rmusic-workshop-activity',active:false,fps:60}});assert.equal(timers.size,0);assert.equal(frames.size,0);assert.equal(draws,1);
});

test('Workshop maintains the requested foreground cadence without timer + display-link double throttling',()=>{
 for (const fps of [24,30,60]) {
  const c=context('file:','file://'),frames=new Map(),events={};let id=0,draws=0;
  c.window={devicePixelRatio:2,requestAnimationFrame:cb=>{frames.set(++id,cb);return id;},cancelAnimationFrame:key=>frames.delete(key),addEventListener:(kind,cb)=>events[kind]=cb,dispatchEvent:()=>{}};
  vm.runInContext(fs.readFileSync(path.join(root,'presentation.js'),'utf8'),c);
  vm.runInContext(fs.readFileSync(path.join(root,'workshop/scheduler.js'),'utf8'),c);
  function draw(){draws++;c.window.requestAnimationFrame(draw);}
  c.window.requestAnimationFrame(draw);c.window.rmusicWorkshopActivity(true,fps);
  for(let time=0;time<1000;time+=16){const work=[...frames.values()];frames.clear();for(const cb of work)cb(time);}
  assert(Math.abs(draws-fps)<=1,`${fps}: ${draws}`);assert.equal(frames.size,1);
  c.window.rmusicWorkshopActivity(false,fps);assert.equal(frames.size,0);
  c.window.rmusicWorkshopActivity(true,fps);assert.equal(frames.size,1);
  events.pagehide();assert.equal(frames.size,0);
 }
});

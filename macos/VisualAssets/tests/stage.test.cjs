const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const root=path.join(__dirname,'../Sources/RMusicVisualAssets/Mineradio');
function script(name){const c=vm.createContext({});vm.runInContext(fs.readFileSync(path.join(root,name),'utf8'),c);return c;}
test('the lyric viewport preserves full background dimensions and remains valid in a small window',()=>{
 const c=script('stage-viewport.js');let r=c.rmusicStageViewport(1200,860,122,134);
 assert.equal(r.width,1200);assert.equal(r.height,604);assert.equal(r.y,134);assert.equal(r.fullHeight,860);
 r=c.rmusicStageViewport(0,10,Infinity,-20);assert(r.height>0&&r.width>0&&r.y===0);
});
test('lyric projection does not crop or duplicate the full-bleed background and restores renderer state',()=>{
 const c=script('stage-viewport.js'),rect=c.rmusicStageViewport(1200,860,122,134),draws=[];
 const camera={aspect:1200/860,layers:{mask:1,set(n){this.mask=1<<n;}},updateProjectionMatrix(){}};
 const renderer={autoClear:true,scissor:false,viewport:[0,0,1200,860],setViewport(...a){this.viewport=a;},setScissor(){},setScissorTest(v){this.scissor=v;},clearDepth(){},render(){draws.push({mask:camera.layers.mask,viewport:[...this.viewport],clear:this.autoClear});}};
 const nodes=[{layers:{set(n){this.value=n;}}}],lyrics={visible:true,traverse(fn){nodes.forEach(fn);}};
 c.rmusicRenderStage(renderer,{},camera,lyrics,rect);
 assert.deepEqual(draws,[{mask:1,viewport:[0,0,1200,860],clear:true},{mask:2,viewport:[0,134,1200,604],clear:false}]);
 assert.equal(nodes[0].layers.value,1);assert.equal(camera.layers.mask,1);assert.equal(camera.aspect,1200/860);assert.equal(renderer.autoClear,true);assert.equal(renderer.scissor,false);assert.deepEqual(renderer.viewport,[0,0,1200,860]);
 renderer.render=()=>{throw Error('lost context');};assert.throws(()=>c.rmusicRenderStage(renderer,{},camera,lyrics,rect));assert.equal(renderer.autoClear,true);assert.equal(renderer.scissor,false);assert.equal(camera.layers.mask,1);
});
test('lyric edge fading shares one DPI-correct uniform, preserves shader hooks and does not recompile every frame',()=>{
 const c=script('stage-viewport.js');let calls=0;
 const material={userData:{},onBeforeCompile(shader){calls++;shader.uniforms.existing={value:1};},customProgramCacheKey(){return 'original';}};
 const shader={uniforms:{},fragmentShader:'void main() { gl_FragColor = vec4(1.0); }'};
 c.rmusicFadeLyricEdges(material);const compile=material.onBeforeCompile;material.needsUpdate=false;
 c.rmusicFadeLyricEdges(material);assert.equal(material.onBeforeCompile,compile);assert.equal(material.needsUpdate,false);
 material.onBeforeCompile(shader,{});assert.equal(calls,1);assert.equal(shader.uniforms.existing.value,1);assert(shader.fragmentShader.includes('gl_FragColor.a *= smoothstep'));assert(material.customProgramCacheKey().startsWith('original:'));
 const camera={aspect:1,layers:{mask:1,set(){}},updateProjectionMatrix(){}};
 const renderer={getPixelRatio(){return 2;},render(){},clearDepth(){},setViewport(){},setScissor(){},setScissorTest(){}};
 c.rmusicRenderStage(renderer,{},camera,{visible:true,traverse(fn){fn({layers:{set(){}},material});}},c.rmusicStageViewport(1200,800,150,134));
 const fade=shader.uniforms.rmusicLyricViewportFade.value;
 assert.equal(fade[0],268);assert.equal(fade[1],1300);assert(Math.abs(fade[2]-82.56)<0.001);
});
test('drag changes only the enabled stage, suppresses queue clicks, cancels on background and removes handlers',()=>{
 const c=script('stage-interaction.js'),events=new Map(),moves=[];let enabled=true,begins=0,ends=0,resets=0;
 const target={addEventListener(n,fn){events.set(n,fn);},removeEventListener(n){events.delete(n);}};
 const dispose=c.rmusicStageInteraction(target,{enabled:()=>enabled,begin:()=>begins++,end:()=>ends++,move:(x,y)=>moves.push([x,y]),reset:()=>resets++});
 events.get('pointerdown')({button:2});assert.equal(begins,0);
 events.get('pointerdown')({button:0,pointerId:1,clientX:10,clientY:20});events.get('pointermove')({pointerId:1,clientX:90,clientY:50});assert.deepEqual(moves,[[80,30]]);
 let suppressed=false;events.get('pointerup')();events.get('click')({stopImmediatePropagation(){suppressed=true;}});assert(suppressed);
 events.get('pointerdown')({button:0,pointerId:1,clientX:0,clientY:0});events.get('blur')();events.get('pointermove')({pointerId:1,clientX:100,clientY:100});assert.equal(moves.length,1);
 enabled=false;events.get('pointerdown')({button:0});events.get('dblclick')();assert.equal(begins,2);assert.equal(resets,0);
 enabled=true;events.get('dblclick')();assert.equal(resets,1);dispose();assert.equal(events.size,0);assert(ends>=3);
});
test('wheel normalizes trackpad and mouse units, defers to the shelf and ignores background input',()=>{
 const c=script('stage-interaction.js'),events=new Map(),zooms=[];let enabled=true,prevented=0;
 const target={addEventListener(n,fn){events.set(n,fn);},removeEventListener(n){events.delete(n);}};
 const dispose=c.rmusicStageInteraction(target,{enabled:()=>enabled,begin(){},end(){},move(){},reset(){},zoom:(delta,event)=>{if(event.shelf)return false;zooms.push(delta);return true;}});
 const wheel=data=>events.get('wheel')({...data,preventDefault(){prevented++;}});
 wheel({deltaY:12,deltaMode:0});wheel({deltaY:-3,deltaMode:1});wheel({deltaY:10,deltaMode:2});
 wheel({deltaY:10,shelf:true});enabled=false;wheel({deltaY:10});assert.deepEqual(zooms,[12,-48,400]);assert.equal(prevented,3);dispose();
});

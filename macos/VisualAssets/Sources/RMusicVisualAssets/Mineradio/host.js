/* R-Music adapter: bounded input, one animation owner, demand rendering, deterministic teardown. */
'use strict';
(()=>{
  let disposed=false,active=false,settling=0,draws=0,timer=0,raf=0,last=0,time=0,fps=60,dirty=true,coverGeneration=0,lastCoverData="",hadCover=false,coverBlend=1;
  let target=new Array(8).fill(0),shown=target.slice(),energy=0,peak=0,lastAudio=0;
  let workshop=null,skullLoading=false,workshopDirty=false,stageViewport;
  let renderWidth=0,renderHeight=0,renderDpr=0,diagnosticAt=0,frameGaps=[],lyricWaitFrames=0;
  let workshopOrbit={x:150,y:30,distance:80,manual:false};shelfManager=makeShelfManager();
  let lastShelfWindow="";
  let settings={preset:0,fps:60,particles:180,scale:1,intensity:0.85,visible:true};
  const signal=(name,value)=>{if(parent!==window)parent.postMessage({type:'rmusic-visual-status',name,value},location.origin);if(window.webkit?.messageHandlers?.visualStatus)window.webkit.messageHandlers.visualStatus.postMessage({name,value});};
  const workshopPost=(type,value={})=>{if(workshop)workshop.contentWindow.postMessage({type,...value},location.protocol==='file:'?'*':location.origin);};
  const workshopSync=()=>{workshopPost('rmusic-workshop-activity',{active:settings.visible&&(active||settling>0),fps,scale:settings.scale});workshopPost('mineradio-sonic-workshop-properties',{properties:{...rmusicWorkshopProperties(fx,stageLyrics.coverPalette),gridSize:settings.particles===80?80:settings.particles===180?140:100,audioIntensity:settings.intensity,showPlayerController:false,showAlbumCover:false,cameraAngleX:workshopOrbit.x,cameraAngleY:workshopOrbit.y,cameraDistance:workshopOrbit.distance,autoRotateEnabled:!workshopOrbit.manual}});};
  window.MineradioSonicWorkshop={pushProperties:()=>{workshopDirty=true;invalidate();}};
  const reportShelfWindow=()=>{const indices=settings.visible&&settings.immersive&&settings.shelf?shelfManager.getCards().map(c=>playQueue[c.item?.queueIndex]?.coreIndex).filter(Number.isInteger).slice(0,11):[];const key=indices.join(",");if(key!==lastShelfWindow){lastShelfWindow=key;signal("queueWindow",indices);}};
  const resize=()=>{const dpr=Math.min(devicePixelRatio||1,1.35,Math.sqrt(5200000/Math.max(1,innerWidth*innerHeight)))*clampRange(settings.scale,0.5,1);if(renderWidth!==innerWidth||renderHeight!==innerHeight||renderDpr!==dpr){renderer.setPixelRatio(dpr);renderer.setSize(innerWidth,innerHeight);renderWidth=innerWidth;renderHeight=innerHeight;renderDpr=dpr;}camera.aspect=innerWidth/innerHeight;camera.updateProjectionMatrix();stageViewport=rmusicStageViewport(innerWidth,innerHeight,settings.immersive?settings.stageTop:0,settings.immersive?settings.stageBottom:0);uniforms.uPixel.value=dpr;dirty=true;};
  const stop=()=>{pacer.reset();if(lyricPaletteTween?.raf)cancelAnimationFrame(lyricPaletteTween.raf);lyricPaletteTween=null;clearTimeout(timer);cancelAnimationFrame(raf);timer=raf=0;last=0;settling=0;};
  const queue=()=>{if(disposed||raf||(!active&&settling<=0)||!settings.visible||document.hidden)return;raf=requestAnimationFrame(draw);};
  const invalidate=(frames=12)=>{const starting=settling===0;settling=Math.max(settling,frames);if(!active&&starting)workshopPost('rmusic-workshop-activity',{active:settings.visible,fps,scale:settings.scale});queue();};
  let sourceSession="",sourceQueueIndex=-1,lyricIndex=-1,sourceClock={position:0,playing:false},lyricDelay=0,lastFeature=-1,previousLow=0;
  const pacer=rmusicFramePacer(),presentation=rmusicPresentationClock(),featureTimeline=rmusicFeatureTimeline(),lyricScheduler=rmusicLyricScheduler();
  const clock=()=>presentation.read();
  function lyricPayload(index){const offsets=lyricDisplayOffsetsForMode(fx.lyricDisplayMode),start=Math.max(0,index+offsets[0]),end=Math.min(lyricsLines.length,index+offsets[offsets.length-1]+1);const entries=lyricsLines.slice(start,end).map((_,i)=>stageLyricContextEntry(start+i,index));const payload=applyLyricTranslationModeToEntries(entries,index-start,20);return {mode:fx.lyricDisplayMode,entries:payload.entries,activeLine:payload.activeLine,key:sourceSession+':'+index};}
  function releaseLyrics(){lyricScheduler.clear();lyricIndex=-1;if(stageLyrics.group)clearStageLyrics();while(lyricDisposeQueue.length)flushLyricDisposeQueue();if(settings.diagnostics)signal('metrics',{event:'lyricsReleased',lyricResources:rmusicLyricResources.metrics(),textures:renderer.info.memory.textures,geometries:renderer.info.memory.geometries});}
  function draw(now){raf=0;if(disposed||!settings.visible||document.hidden)return;if((active||settling>0)&&!pacer.due(now,fps)){queue();return;}const rawGap=last?now-last:1000/fps;const dt=Math.min(0.08,rawGap/1000);last=now;draws++;if(settings.diagnostics&&dt>0)frameGaps.push(rawGap);if(frameGaps.length>240)frameGaps.shift();if(!active)settling=Math.max(0,settling-1);frameDt=frameDelta=dt;if(active)time+=dt;
    beatPulse*=Math.exp(-dt*5);
    for(const due of featureTimeline.drain(clock())){lastFeature=due.position;applyAudio(due);}
    const feature=featureTimeline.sample(clock());
    const fresh=playing&&feature!==null&&performance.now()-lastAudio<350;
    for(let i=0;i<8;i++){const aim=fresh?target[i]:0;shown[i]+=(aim-shown[i])*(1-Math.exp(-dt*(aim>shown[i]?45:12)));}
    bass=(shown[0]+shown[1])*0.5;mid=(shown[2]+shown[3]+shown[4])/3;treble=(shown[5]+shown[6]+shown[7])/3;
    uniforms.uTime.value=time;uniforms.uVinylSpin.value=time*0.14;uniforms.uBass.value=bass;uniforms.uMid.value=mid;uniforms.uTreble.value=treble;uniforms.uEnergy.value=fresh?energy:0;uniforms.uBeat.value=beatPulse;
    if(active)coverBlend=Math.min(1,coverBlend+dt/0.52);uniforms.uColorMixT.value=visualEase(coverBlend);uniforms.uAlpha.value=uniforms.uHasCover.value>0.5?1:0;uniforms.uIntensity.value=settings.intensity;uniforms.uPreset.value=settings.preset;
    particles.position.x=bloomParticles.position.x=0;particles.scale.setScalar(1);bloomParticles.scale.setScalar(1);
    pointerParallax.x+=(pointerTarget.x-pointerParallax.x)*0.04;pointerParallax.y+=(pointerTarget.y-pointerParallax.y)*0.04;
    updateCinema(dt);updateCamera();if(settings.immersive&&settings.shelf){shelfManager.update(dt);reportShelfWindow();}
    particles.rotation.x=bloomParticles.rotation.x=particleRotation.x+pointerParallax.y*0.04;
    particles.rotation.y=bloomParticles.rotation.y=particleRotation.y+pointerParallax.x*0.04;
    audioEnergy=fresh?energy:0;lyricSunEnergy+=(audioEnergy*0.25-lyricSunEnergy)*0.08;
    if(settings.immersive&&lyricsLines.length&&audio){
      const position=clock()-lyricDelay;let next=-1;for(let i=0;i<lyricsLines.length;i++){if(lyricsLines[i].time>position)break;next=i;}
      if(next>=0&&next!==lyricIndex){const payload=lyricPayload(next),prepared=lyricScheduler.take(payload);
        if(prepared){const previousLyricIndex=lyricIndex;stageLyrics.transitionLineStep=previousLyricIndex<0?0:clampRange(next-previousLyricIndex,-2,2);showStageLine(payload,!active,{preparedMesh:prepared});lyricIndex=next;if(!active&&stageLyrics.current){stageLyrics.current.userData.age=1;primeLyricMeshOpacity(stageLyrics.current,1);}if(next+1<lyricsLines.length)lyricScheduler.prepare(lyricPayload(next+1));}
        else {lyricWaitFrames++;lyricScheduler.prepare(payload,true);}
      }else if(next<0&&lyricIndex<0)lyricScheduler.prepare(lyricPayload(0));
      if(lyricIndex>=0&&stageLyrics.current){const line=lyricsLines[lyricIndex];updateLyricMeshProgress(stageLyrics.current,getLyricLineProgress(line,lyricsLines[lyricIndex+1],position),{nativeKaraoke:lyricLineHasNativeKaraoke(line)});}
    }
    if(stageLyrics.group){stageLyrics.group.visible=!!settings.immersive;const aspect=camera.aspect;camera.aspect=stageViewport.width/stageViewport.height;camera.updateProjectionMatrix();updateStageLyrics3D(dt);camera.aspect=aspect;camera.updateProjectionMatrix();}
    uniforms.uBurstAmt.value*=Math.exp(-dt*6);updateRipples(dt);updateFloatLayer(dt);
    const main=![3,6,8].includes(settings.preset)&&!([5,7].includes(settings.preset)&&fx.backgroundStarRiver===false);particles.visible=main;bloomParticles.visible=main&&fx.bloom&&fx.bloomStrength>0.01;
    if(floatGroup)floatGroup.visible=![6,8].includes(settings.preset);if(backCoverGroup)backCoverGroup.visible=![6,8].includes(settings.preset);
    backgroundStarRiverUniforms.uTime.value=time;updateBackgroundStarRiverState(dt,true);
    updateSkullParticleLayer(dt);
    MineradioSonicTopography.update(dt,{fx,scene,time,audio:rmusicSonicAudio(shown,fresh?energy:0,beatPulse)});
    MineradioSonicTopography.place(0);
    if(workshopDirty){workshopSync();workshopDirty=false;}
    if(!active&&settling===0)workshopPost('rmusic-workshop-activity',{active:false,fps,scale:settings.scale});
    if(settings.diagnostics&&now-diagnosticAt>=5000){diagnosticAt=now;const gaps=frameGaps.slice().sort((a,b)=>a-b);signal('metrics',{active,playing,draws,time,position:clock(),featurePosition:lastFeature,featureAge:feature?clock()-feature.position:null,audioEnergy:fresh?energy:0,coverPrimary:stageLyrics.coverPalette?.rawPrimary,shelfCovers:Object.values(playlistCoverCache).filter(e=>e.loaded).length,lyricIndex,lyricWaitFrames,lyricResources:rmusicLyricResources.metrics(),textures:renderer.info.memory.textures,geometries:renderer.info.memory.geometries,frameP95:gaps[Math.floor(gaps.length*.95)]||0,frameMax:gaps[gaps.length-1]||0,build:lyricScheduler.metrics(),pendingBuild:lyricScheduler.pending()});}
    renderer.setClearAlpha(settings.preset===8?0:1);rmusicRenderStage(renderer,scene,camera,stageLyrics.group,stageViewport);dirty=false;queue();
  }
  function applyAudio(value){const a=Array.isArray(value.bands)?value.bands:[];target=Array.from({length:8},(_,i)=>clampRange(a[i],0,255)/255);energy=clampRange(value.energy,0,255)/255;peak=clampRange(value.peak,0,255)/255;const low=(target[0]+target[1])*0.5,onset=Math.max(0,low-previousLow);previousLow=low;const age=Math.max(0,clock()-value.position);if(onset>0.06&&age<=0.3){beatPulse=Math.max(beatPulse,Math.min(1,onset*3)*Math.exp(-age*5));scheduleBeatCamera({time:clock(),strength:beatPulse,confidence:0.75,low:target[0],body:target[1],impact:beatPulse,primary:true},'live');}lastAudio=performance.now();if(workshop){const samples=Array.from({length:512},(_,i)=>target[Math.min(7,Math.floor(i/64))]*0.45);workshop.contentWindow.postMessage({type:'mineradio-sonic-workshop-audio',samples},location.protocol==='file:'?'*':location.origin);}}
  window.rmusicVisual={
    redraw(){invalidate(4);},
    configure(value){if(disposed)return;const previous=settings.preset,previousShelf=settings.shelf,previousParameters=settings.parameters;settings={...settings,...value};settings.preset=clampRange(settings.preset,0,12);visualHostVisible=settings.visible!==false;if(!visualHostVisible||!settings.immersive)releaseLyrics();
      if(previousParameters!==settings.parameters){
        let overrides={};try{if(typeof settings.parameters==='string'&&settings.parameters.length<=16384)overrides=JSON.parse(settings.parameters);}catch{}
        for(const field of visualControlSchema){let value=overrides?.[field.id];
          if(field.kind==='range')value=typeof value==='number'&&Number.isFinite(value)?clampRange(value,field.minimum,field.maximum):field.number;
          else if(field.kind==='toggle')value=typeof value==='boolean'?value:field.flag;
          else if(field.kind==='choice')value=field.choices.some(c=>c.id===value)?value:field.text;
          else if(field.kind==='color')value=typeof value==='string'&&/^#[a-f0-9]{6}$/i.test(value)?value:field.text;
          fx[field.id]=value;
        }
        releaseLyrics();if(settings.shelf)shelfManager.rebuild(false);
      }
      fps=[24,30,60].includes(settings.fps)?settings.fps:30;settings.particles=[80,118,180].includes(settings.particles)?settings.particles:118;
      fx.sonicGroundDensity=settings.particles===80?16:settings.particles===180?52:32;fx.sonicGroundFloatingCount=settings.particles===80?12:settings.particles===180?40:24;
      fx.preset=settings.preset;if(previous!==settings.preset)applyPresetOrbitBaseline(settings.preset,{syncCurrent:true});fx.intensity=clampRange(settings.intensity,0,1.5);settings.intensity=fx.intensity;
      if(geo.userData.grid!==settings.particles)applyCoverParticleResolution(settings.particles/118,{reload:false});
      MineradioSonicTopography.onPresetChange(previous,settings.preset,{scene,fx});
      if(previous===6&&settings.preset!==6&&skullParticleGroup){scene.remove(skullParticleGroup);skullParticleGroup.geometry.dispose();skullParticleGroup.material.dispose();skullParticleGroup=null;skullParticleAsset.data=null;}
      if(settings.preset===6&&!skullParticleAsset.data&&!skullLoading){skullLoading=true;const script=document.createElement('script');script.src='skull-data.js';script.onload=()=>{skullLoading=false;if(settings.preset===6)window.rmusicVisual.skull(window.rmusicSkullData);window.rmusicSkullData=null;script.remove();};document.body.appendChild(script);}
      if(settings.preset!==8&&workshop){workshopPost('rmusic-workshop-activity',{active:false,fps});workshop.remove();workshop=null;}
      fx.floatLayer=!!settings.floatLayer;fx.backCover=!!settings.backCover;if(fx.floatLayer&&!floatGroup){createFloatLayer();if(coverPickerCanvas)refreshFloatColorsFromCover(coverPickerCanvas);}else if(!fx.floatLayer&&floatGroup)destroyFloatLayer();if(fx.backCover&&!backCoverGroup){createBackCoverLayer();if(coverPickerCanvas)refreshBackCoverColorsFromCanvas(coverPickerCanvas);}else if(!fx.backCover&&backCoverGroup)destroyBackCoverLayer();uniforms.uFloatAlpha.value=fx.floatLayer?1:0;fx.shelfPresence='always';shelfPinnedOpen=!!settings.shelf;shelfManager.setMode(settings.immersive&&settings.shelf?'side':'off');if(settings.immersive&&settings.shelf&&!previousShelf)activateFocusZone('shelf-side');else if(!settings.immersive||!settings.shelf){orbit.focus.active=false;orbit.centerLocked=true;}syncFxUniforms();workshopSync();reportShelfWindow();resize();invalidate();
    },
    activity(value){if(disposed)return;active=Boolean(value)&&!document.hidden;visualHostActive=active;workshopSync();if(!active){stop();target.fill(0);featureTimeline.clear();lastFeature=-1;previousLow=0;lastAudio=0;invalidate();}else queue();},
    metadata(value){if(disposed||!value)return;const paused=playing&&!value.playing;if(paused){target.fill(0);featureTimeline.clear();lastFeature=-1;previousLow=0;lastAudio=0;if(workshop)workshopPost('mineradio-sonic-workshop-audio',{samples:new Array(512).fill(0)});}const jumped=Math.abs((Number(value.position)||0)-clock())>1.25;const session=String(value.session||value.id||'');const changed=currentSongKey()!==value.id||session!==sourceSession;sourceSession=session;sourceQueueIndex=Number.isInteger(value.queueIndex)?value.queueIndex:-1;sourceClock={position:Math.max(0,Number(value.position)||0),playing:!!value.playing};presentation.set(value);lyricDelay=Math.max(-30,Math.min(30,Number(value.lyricOffset)||0));playing=!!value.playing;playlist=[{id:value.id,name:String(value.title||''),artists:[{name:String(value.artist||'')}]}];audio={src:value.id||'',get currentTime(){return clock();},get paused(){return !playing;},ended:false,duration:Number(value.duration)||0};if(changed||jumped){currentIdx=Math.max(0,playQueue.findIndex(s=>sourceQueueIndex>=0?s.coreIndex===sourceQueueIndex:s.id===value.id));target.fill(0);shown.fill(0);energy=peak=0;lastAudio=0;resetBeatCameraSync(sourceClock.position);featureTimeline.clear();lastFeature=-1;previousLow=0;shelfManager.rebuild(false);releaseLyrics();}invalidate();},
    lyrics(value){if(disposed)return;releaseLyrics();lyricsLines=Array.isArray(value)?value.slice(0,2000).filter(line=>Number.isFinite(line.time)&&line.time>=0&&typeof line.text==='string'&&line.text.length<=2000).map(line=>{let offset=0;const input=Array.isArray(line.words)?line.words.slice(0,500).filter(w=>Number.isFinite(w.time)&&w.time>=0&&typeof w.text==='string'&&w.text.length<=2000):[];const words=input.map((w,i)=>{const c0=offset;offset+=w.text.length;return {t:w.time,d:Number.isFinite(w.duration)&&w.duration>=0?w.duration:0,c0,c1:offset};});return {...line,t:line.time,words,charCount:line.text.length};}):[];lyricPrimaryVirtualPrefixCache={key:'',values:[0]};lyricIndex=-1;invalidate();},
    audio(value){if(disposed||!active||!playing||!settings.visible)return;if(Array.isArray(value.frames)){featureTimeline.push(value.frames,sourceSession);}else if(Array.isArray(value.bands)&&value.bands.length===8){featureTimeline.push([{...value,position:clock(),playback_id:sourceSession}],sourceSession);}else featureTimeline.clear();},
    queue(value){if(disposed)return;const next=rmusicShelfQueue(value),same=next.length===playQueue.length&&next.every((s,i)=>s.id===playQueue[i].id&&s.coreIndex===playQueue[i].coreIndex);playQueue=next;prunePlaylistCovers(playQueue.map(s=>s.cover));currentIdx=Math.max(0,playQueue.findIndex(s=>sourceQueueIndex>=0?s.coreIndex===sourceQueueIndex:s.id===currentSongKey()));if(same)shelfManager.refreshCovers();else shelfManager.rebuild(false);invalidate();},
    scroll(value){if(disposed)return;if(settings.shelf)shelfManager.scrollBy(Number(value)>0?1:-1);invalidate();},
    select(value){if(disposed)return;if(!settings.shelf)return;const hit=shelfManager.pickCardAtScreen(Number(value.x),Number(value.y),0);if(hit?.card){const action=hit.card.mesh.userData.action;if(action?.kind==='playQueue')playQueueAt(action.index);}},
    pointer(value){if(disposed)return;pointerTarget.x=clampRange(value?.x,-1,1);pointerTarget.y=clampRange(value?.y,-1,1);uniforms.uMouseXY.value.set(pointerTarget.x*2.4,-pointerTarget.y*2.4);uniforms.uMouseActive.value=1;},
    cover(data){if(disposed)return;if(data===lastCoverData)return;if(data&&(typeof data!=="string"||!data.startsWith("data:image/")||data.length>1048576))return;lastCoverData=data;const generation=++coverGeneration;if(!data){uniforms.uHasCover.value=0;return;}const image=new Image();image.onload=()=>{if(disposed||generation!==coverGeneration)return;const old=coverTex.image;if(hadCover){prevCoverTex.image=old;prevCoverTex.needsUpdate=true;}coverBlend=hadCover&&active?0:1;hadCover=true;const canvas=document.createElement('canvas');canvas.width=canvas.height=256;canvas.getContext('2d').drawImage(image,0,0,256,256);coverTex.image=canvas;coverTex.needsUpdate=true;coverPickerCanvas=canvas;uniforms.uHasCover.value=1;uniforms.uColorMixT.value=visualEase(coverBlend);coverEdgeTex.image=buildEdgeAndDepth(canvas);coverEdgeTex.needsUpdate=true;uniforms.uHasDepth.value=1;uniforms.uAiBoost.value=0.55;uniforms.uEdgeEnabled.value=fx.edge?1:0;if(floatGroup)refreshFloatColorsFromCover(canvas);if(backCoverGroup)refreshBackCoverColorsFromCanvas(canvas);updateLyricPaletteFromCover(canvas);dirty=true;invalidate();};image.src=data;},
    skull(data){if(disposed||!data)return;const text=atob(data),bytes=new Uint8Array(text.length);for(let i=0;i<text.length;i++)bytes[i]=text.charCodeAt(i);if(bytes.length%20===0)skullParticleAsset.data=new Float32Array(bytes.buffer);invalidate();},
    dispose(){if(disposed)return;disposed=true;active=false;stop();releaseLyrics();featureTimeline.clear();disposeInteraction();if(workshop){workshopPost('rmusic-workshop-activity',{active:false,fps});workshop.remove();workshop=null;}coverGeneration++;if(lyricPaletteTween?.raf)cancelAnimationFrame(lyricPaletteTween.raf);lyricPaletteTween=null;if(stageLyrics.group)disposeLyricsParticles();shelfManager.setMode('off');prunePlaylistCovers([]);playlistCoverCache=Object.create(null);playQueue=[];MineradioSonicTopography.clear();const geometries=new Set(),materials=new Set(),textures=new Set();scene.traverse(node=>{if(node.geometry)geometries.add(node.geometry);if(node.material){for(const m of Array.isArray(node.material)?node.material:[node.material]){materials.add(m);if(m.uniforms)for(const u of Object.values(m.uniforms))if(u.value?.isTexture)textures.add(u.value);}}});geometries.forEach(g=>g.dispose());materials.forEach(m=>m.dispose());textures.forEach(t=>t.dispose());skullParticleAsset.data=null;renderer.dispose();renderer.forceContextLoss();scene.clear();lastCoverData="";lyricsLines=[];playlist=[];audio=null;target=shown=[];}
  };
  // Native sends the pinned local asset; this view has no fetch permission.
  loadSkullParticleAsset=()=>Promise.resolve(skullParticleAsset.data);
  window.addEventListener('message',event=>{if(event.source===parent&&event.origin===location.origin&&event.data?.type==='rmusic-visual-diagnostics'&&new URLSearchParams(location.search).has('diagnostics'))parent.postMessage({type:'rmusic-visual-diagnostics',active,time,draws,info:renderer.info.memory,calls:renderer.info.render.calls,cards:shelfManager.getCards().length,lyrics:stageLyrics.outgoing.length+(stageLyrics.current?1:0),queued:playQueue.length},location.origin);});
  window.addEventListener('message',event=>{if(event.source!==parent||event.origin!==location.origin||event.data?.type!=='rmusic-visual')return;const {kind,value}=event.data;if(['configure','activity','audio','cover','metadata','lyrics','pointer','queue','select','scroll','dispose'].includes(kind))window.rmusicVisual[kind](value);});
  window.addEventListener('message',event=>{if(event.source!==workshop?.contentWindow)return;if(event.data?.type==='mineradio-sonic-workshop-ready')workshopSync();if(event.data?.type==='rmusic-workshop-error')signal('rendererError',String(event.data.message||'Workshop failed').slice(0,4000));});
  window.addEventListener('resize',resize);document.addEventListener('visibilitychange',()=>{if(document.hidden){active=false;stop();}});
  window.addEventListener('pagehide',()=>window.rmusicVisual.dispose(),{once:true});
  window.addEventListener('wheel',event=>{if(event.clientX>innerWidth*0.65)window.rmusicVisual.scroll(event.deltaY);},{passive:true});
  window.addEventListener('click',event=>window.rmusicVisual.select({x:event.clientX,y:event.clientY}));
  window.addEventListener('pointermove',event=>{pointerTarget.x=(event.clientX/innerWidth-0.5)*2;pointerTarget.y=(event.clientY/innerHeight-0.5)*2;});
  const disposeInteraction=rmusicStageInteraction(window,{
    enabled:()=>!disposed&&settings.immersive&&settings.visible,
    begin(){unlockCenteredView();orbit.focus.active=false;orbit.rotating=true;orbit.recentering=false;},
    move(dx,dy){orbit.userTheta-=dx*0.006;orbit.userPhi=clampRange(orbit.userPhi+dy*0.005,orbit.minPhi,orbit.maxPhi);workshopOrbit.x=(workshopOrbit.x-dx*0.25+360)%360;workshopOrbit.y=clampRange(workshopOrbit.y+dy*0.2,5,85);workshopOrbit.manual=true;workshopDirty=true;invalidate();},
    end(){orbit.rotating=false;},
    zoom(delta,event){if(settings.shelf&&event.clientX>innerWidth*0.65)return false;unlockCenteredView();orbit.focus.active=false;orbit.userRadius=clampRange(orbit.userRadius+delta*0.005,orbit.minRadius,orbit.maxRadius);workshopOrbit.distance=clampRange(workshopOrbit.distance+delta*0.08,20,180);workshopDirty=true;invalidate();return true;},
    reset(){if(!applyPresetOrbitBaseline(settings.preset,{syncCurrent:true})){const base=defaultOrbitStateForPreset(settings.preset);orbit.userTheta=orbit.theta=orbit.baselineTheta=base.theta;orbit.userPhi=orbit.phi=orbit.baselinePhi=base.phi;orbit.userRadius=orbit.radius=orbit.baselineRadius=base.radius;}workshopOrbit={x:150,y:30,distance:80,manual:false};workshopDirty=true;invalidate();}
  });
  renderer.domElement.addEventListener('webglcontextlost',()=>{stop();signal('contextLost',true);});
  applyPresetOrbitBaseline(fx.preset,{syncCurrent:true});syncFxUniforms();resize();draw(performance.now());signal('ready',true);
})();

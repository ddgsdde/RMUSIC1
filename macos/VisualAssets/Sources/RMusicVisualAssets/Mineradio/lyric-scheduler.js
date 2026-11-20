/* Prepare just the next boundary in cooperative chunks. No full-song GPU cache. */
'use strict';
function rmusicLyricScheduler() {
  let job=null,timer=0,prepared=null,preparedKey='',generation=0,maxChunkMs=0; const now=()=>typeof performance!=='undefined'?performance.now():Date.now();
  function abort(){generation++;clearTimeout(timer);timer=0;if(job)cancelCooperativeLyricMeshBuild(job.state);job=null;if(prepared)disposeLyricMesh(prepared);prepared=null;preparedKey='';}
  function prepare(payload,urgent=false){if(!payload||preparedKey===payload.key)return;if(job?.payload.key===payload.key){job.urgent=job.urgent||urgent;return;}abort();const token=generation;job={payload,urgent,state:beginCooperativeLyricMeshBuild(payload)};
    function work(){timer=0;if(token!==generation||!job||!visualHostVisible||document.hidden){abort();return;}try{
      const began=now(),done=stepCooperativeLyricMeshBuild(job.state,job.urgent?4:1,job.urgent?3:2);maxChunkMs=Math.max(maxChunkMs,now()-began);if(done){prepared=finishCooperativeLyricMeshBuild(job.state);preparedKey=payload.key;job=null;if(prepared){prepared.visible=false;prepared.traverse(node=>{const materials=node.material?(Array.isArray(node.material)?node.material:[node.material]):[];for(const mat of materials){for(const texture of [mat.map,mat.uniforms?.uMap?.value])if(texture?.userData?.__mineradioLyricOwned)renderer.initTexture(texture);}});}window.rmusicVisual?.redraw();return;}
      timer=setTimeout(work,job.urgent?0:8);
    }catch(error){abort();signalLyricBuildFailure(error);}}
    timer=setTimeout(work,0);
  }
  return {prepare,metrics(){return {maxChunkMs};},clear:abort,take(payload){if(preparedKey!==payload.key)return null;const mesh=prepared;prepared=null;preparedKey='';return mesh;},pending(){return !!job;}};
}
function signalLyricBuildFailure(error){console.error('Lyric preparation failed',error);}

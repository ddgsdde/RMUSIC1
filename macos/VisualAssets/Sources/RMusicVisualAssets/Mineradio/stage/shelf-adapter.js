// Core owns the queue and actions. Upstream shelf receives metadata only and allocates at most eleven cards.
var userPlaylists=[],myPodcastCollections=[],playlistCatalogRevision=0,playQueue=[],playlistCoverCache=Object.create(null);
var shelfOpenAnimAt=-10,shelfHoverCue={target:0,value:0,x:0,y:0,lastAt:0,enteredAt:0,zoneActive:false,guide:false};
var shelfPlaybackSwitchGuardActive=()=>false,tickShelfHoverCue=()=>0;
var queueItemKey=s=>String(s?.id||'')+':'+String(s?.coreIndex??''),queueCurrentKey=()=>queueItemKey(playQueue[currentIdx]);
var songCoverSrc=s=>s?.cover||'',compactCount=n=>String(n||0);
var normalizeForegroundFpsMode=()=> 'fixed',scheduleUiWarmTask=f=>f();
function rmusicShelfQueue(value) {
  return Array.isArray(value)?value.slice(0,50).map(s=>({id:String(s.id||''),name:String(s.title||'').slice(0,500),artist:String(s.artist||'').slice(0,500),coreIndex:Number(s.index),cover:typeof s.cover==='string'&&s.cover.length<65536&&/^data:image\/jpeg;base64,/.test(s.cover)?s.cover:''})):[];
}
function prunePlaylistCovers(sources) {
  const keep=new Set(sources.filter(Boolean).slice(0,11));
  for(const src of Object.keys(playlistCoverCache))if(!keep.has(src)) {
    const entry=playlistCoverCache[src];entry.img.onload=entry.img.onerror=null;entry.img.src='';delete playlistCoverCache[src];
  }
}
var requestPlaylistCover=(src,done)=>{
  if(!/^data:image\//.test(src)||playlistCoverCache[src]||Object.keys(playlistCoverCache).length>=11)return;
  const img=new Image(),entry={img,loaded:false,loading:true};playlistCoverCache[src]=entry;
  img.onload=()=>{if(playlistCoverCache[src]!==entry)return;entry.loaded=true;entry.loading=false;done();};
  img.onerror=()=>{if(playlistCoverCache[src]!==entry)return;entry.failed=true;entry.loading=false;done();};
  img.src=src;
};
var playQueueAt=index=>{const item=playQueue[index];if(!item)return;const value={name:'queueSelect',value:{index:item.coreIndex,id:item.id}};window.webkit?.messageHandlers?.visualStatus?.postMessage(value);if(parent!==window)parent.postMessage({type:'rmusic-visual-action',...value},location.origin);};

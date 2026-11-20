var visualHostVisible=true,visualHostActive=false;
/* R-Music owns playback. The original visual functions only see this read-only clock and metadata. */
var BASE_FOV=45,camPunch=0,djMode={active:false},frameDt=1/60,frameDelta=1/60;
var audio=null,playing=false,currentIdx=0,playlist=[],lyricsLines=[],lyricsTranslationLines=[],lyricsHasNativeKaraoke=false;
var lyricSunEnergy=0,lyricSunTarget=0,lyricSunHold=0,lyricSunAvg=0,lyricSunPeak=0.55;
var audioEnergy=0,sonicAudioFrame=null,currentBeatMap=null,currentDjBeatMap=null,beatMapNextIdx=0;
var pointerParallax={x:0,y:0},pointerTarget={x:0,y:0},headNeutral=null,particleRotation={x:0,y:0},particleSpin={vx:0,vy:0};
var freeCameraState=null,customLyricFonts=[],trackSwitchToken=0,lyricsVisible=true,lyricOffset=0;
var runtimePerfBudgetLevel=()=>2,runtimePerfScale=()=>1,lyricPerfBudgetScale=()=>1,lyricTextureScale=()=>1;
var normalizeForegroundFpsMode=()=> 'fixed',isProgressDragPreviewActive=()=>false,isDeepBackgroundMode=()=>document.hidden||!visualHostVisible||!visualHostActive;
var isRenderInteractionActive=()=>false,markRenderInteraction=()=>{},getLyricFontBudget=()=>1;
var notifyVisualSettingsChanged=()=>{},saveFx=()=>{},updateCursor=()=>{},showToast=()=>{};
var readSavedFreeCameraState=()=>null,readSavedVolume=()=>1,readSavedLyricLayout=()=>({});
var clamp01=v=>Math.min(1,Math.max(0,Number(v)||0));
var stageLyricPlaybackSeconds=()=>audio?.currentTime||0;
var currentSongKey=()=>playlist[0]?.id||'',songKey=s=>s?.id||'';
var currentLyricFallbackText=()=>playlist[0]?.name||'',escapeHtml=s=>String(s).replace(/[<>&"']/g,'');
var desiredLyricRenderSignature=()=>currentSongKey(),currentAppliedLyricRenderSignature=()=>currentSongKey();
var isDesktopLyricsEnabled=()=>false,isDesktopWallpaperMode=()=>false,isDesktopWallpaperInteractive=()=>false;
var getAdjustedLyricPlaybackTime=t=>t,stageLyricProgressPreviewActive=()=>false;
var sonicPresetCameraActive=()=>fx.preset===7,hasAnyPlatformLogin=()=>false;
var shelfAlwaysVisible=()=>false,shelfSummonSettings=()=>({cameraEnterSpeed:1,cameraExitSpeed:1});
var updateStageLyricStatus=()=>{},scheduleLyricLayoutSave=()=>{},isDevelopmentLockedFx=()=>false;

// Visibility is independent of the paused transport.
function isHiddenForBackgroundOptimization() { return document.hidden || !visualHostVisible; }

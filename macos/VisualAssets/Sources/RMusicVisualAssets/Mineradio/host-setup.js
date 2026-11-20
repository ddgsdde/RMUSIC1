/* R-Music visual-only host. No credentials, network, audio context or playback objects. */
'use strict';
window.addEventListener('error', event => { document.body.dataset.visualError = event.error?.stack || event.message;window.webkit?.messageHandlers?.visualStatus?.postMessage({name:'rendererError',value:document.body.dataset.visualError}); if(parent!==window)parent.postMessage({type:'rmusic-visual-status',error:document.body.dataset.visualError},location.origin);console.error('Mineradio renderer:', event.error?.stack || event.message); });
var renderer = new THREE.WebGLRenderer({alpha:true,antialias:false,powerPreference:'low-power'});
renderer.setPixelRatio(0.8);renderer.setSize(innerWidth,innerHeight);renderer.setClearColor(0x070a10);document.body.appendChild(renderer.domElement);renderer.domElement.style.cssText='position:relative;z-index:1';
var scene=new THREE.Scene(),camera=new THREE.PerspectiveCamera(48,innerWidth/innerHeight,0.1,100);
camera.position.set(0,0.2,8);camera.lookAt(0,0,0);
var fxDefaults={visualTintColor:'#9db8cf'};
var fx={preset:0,intensity:0.7,coverResolution:1,backgroundStarRiver:true,cinema:false,sonicGroundDensity:32,sonicGroundFloatingCount:24};
var clampRange=(value,lo,hi)=>Math.min(hi,Math.max(lo,Number(value)||0));
var coverParticleGridForResolution=(value)=>Math.round(clampRange(value*118,80,180));
var normalizeCoverResolution=(value)=>clampRange(value,80/118,180/118);
var stageLyrics={palette:{},coverPalette:{}};
var normalizeHexColor=(value,fallback)=>/^#[0-9a-f]{6}$/i.test(String(value))?value:fallback;
var orbit={centerLocked:true,rotating:false,lookAt:new THREE.Vector3()},headParallax={active:false},gestureRotation={x:0,y:0},shelfManager=null,shelfPinnedOpen=false,shelfVisibility=0;
var bass=0,mid=0,treble=0,beatPulse=0;

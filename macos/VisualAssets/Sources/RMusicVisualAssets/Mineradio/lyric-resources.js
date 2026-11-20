/* Ownership accounting includes CPU canvas backing stores, not just WebGL texture handles. */
'use strict';
var rmusicLyricResources=(()=>{
  let bytes=0,peak=0,count=0;
  return {
    own(texture){if(!texture?.userData?.__mineradioLyricOwned||texture.userData.rmusicOwned)return texture;texture.userData.rmusicOwned=true;const canvas=texture.image;const size=Math.ceil((canvas?.width||0)*(canvas?.height||0)*4*(texture.generateMipmaps?7/3:2));bytes+=size;peak=Math.max(peak,bytes);count++;
      const release=()=>{texture.removeEventListener('dispose',release);bytes-=size;count--;if(canvas?.getContext){canvas.width=canvas.height=1;}texture.image=null;};texture.addEventListener('dispose',release);return texture;},
    metrics(){return {bytes,peak,count};}
  };
})();

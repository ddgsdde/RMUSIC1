// R-Music bridge for Mineradio d43de565 (GPL-3.0) region palettes.
// Use the same extracted cover palette as lyrics / topography; never decode audio here.
function rmusicWorkshopProperties(fx, palette) {
  fx=fx||{};palette=palette||{};
  const hex=(value,fallback)=>'#'+new THREE.Color(value||fallback).getHexString();
  const mix=(a,b,t)=>'#'+new THREE.Color(a).lerp(new THREE.Color(b),t).getHexString();
  const custom=fx.visualTintMode==='custom';
  const primary=hex(custom?fx.visualTintColor:fx.sonicWorkshopColorMode==='custom'?fx.sonicWorkshopCustomColor:palette.rawAreaPrimary||palette.rawPrimary,'#9db8cf');
  const region=(role,fallback,key)=>hex(custom?primary:fx[key+'Mode']==='custom'?fx[key]:palette[role],fallback);
  const base=region('rawAreaBase',mix('#000000',primary,.16),'sonicWorkshopBaseColor');
  const cool=region('rawAreaCool',primary,'sonicWorkshopCoolColor'),warm=region('rawAreaWarm',primary,'sonicWorkshopWarmColor');
  return {
    theme:'mineradio-custom',
    mineradioCustomTheme:{name:'Album cover',id:'mineradio-custom',__primaryColor:primary,
      uBaseColor1:mix('#000000',base,.20),uBaseColor2:mix(base,primary,.22),
      uCoolCore:cool,uCoolEdge:mix('#000000',cool,.48),uWarmCore:warm,uWarmEdge:mix('#000000',warm,.56),
      uRippleColor:region('rawAreaLight',primary,'sonicWorkshopRippleColor'),uPeakColor:region('rawAreaAccent',cool,'sonicWorkshopPeakColor'),uGlowIntensity:.8},
    audioIntensity:Number.isFinite(fx.sonicWorkshopAudioIntensity)?fx.sonicWorkshopAudioIntensity:1.15,
    responseRange:Number.isFinite(fx.sonicWorkshopResponseRange)?fx.sonicWorkshopResponseRange:1.3,
    peakColorIntensity:Number.isFinite(fx.sonicWorkshopPeakIntensity)?fx.sonicWorkshopPeakIntensity:.62
  };
}

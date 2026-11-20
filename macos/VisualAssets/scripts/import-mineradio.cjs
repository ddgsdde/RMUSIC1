// Reproduce the vendored visual declarations and native visual control metadata from the pinned checkout.
// Usage: node macos/VisualAssets/scripts/import-mineradio.cjs /path/to/Mineradio-paused
const fs=require('node:fs'),path=require('node:path'),{execFileSync}=require('node:child_process'),{createRequire}=require('node:module');
const revision='d43de565acabfdc1a9c9820a27e81a98ccbebcef';
const root=path.resolve(__dirname,'../../..'),checkout=path.resolve(process.argv[2]||'');
if(!process.argv[2]||execFileSync('git',['-C',checkout,'rev-parse','HEAD'],{encoding:'utf8'}).trim()!==revision)throw Error('Expected the pinned upstream checkout '+revision);
const dependency=createRequire(path.join(root,'apps/web/package.json')),ts=dependency('typescript');
const source=path.join(checkout,'public/js/modules'),destination=path.join(root,'macos/VisualAssets/Sources/RMusicVisualAssets/Mineradio/stage');
fs.mkdirSync(destination,{recursive:true});
const header=`// Mineradio ${revision}, GPL-3.0.\n// R-Music port: selected visual declarations. Upstream DOM events, network and startup hooks excluded.\n`;
function declarations(file,choose){const text=fs.readFileSync(path.join(source,file),'utf8'),ast=ts.createSourceFile(file,text,ts.ScriptTarget.Latest,true,ts.ScriptKind.JS);return ast.statements.filter(s=>choose(s,ts)).map(s=>s.getFullText(ast)).join('\n');}
function selected(file,names,out){fs.writeFileSync(path.join(destination,out),header+declarations(file,s=>ts.isFunctionDeclaration(s)&&names.includes(s.name.text)));}
const files=['01-scene/01-orbit-free-camera.js','01-scene/02-beat-camera-runtime.js','01-scene/03-focus-cinema-camera.js',...['02-lyrics-state-layout','03-lyrics-star-river','05-lyrics-fonts-texture','07-lyrics-palette-text-utils','08-lyrics-display-modes','09-lyrics-payloads','10-lyrics-mask-textures','11-lyrics-shaders','12-lyrics-row-layers','13-lyrics-mesh-build','14-stage-lyrics-rendering','15-ripples-cover-depth'].map(n=>'02-visual/'+n+'.js')];
for(const file of files)fs.writeFileSync(path.join(destination,path.basename(file)),header+declarations(file,s=>ts.isFunctionDeclaration(s)||ts.isVariableStatement(s)||(file.endsWith('15-ripples-cover-depth.js')&&ts.isForStatement(s))));
fs.writeFileSync(path.join(destination,'beat-state.js'),header+declarations('00-state/03-beat-dj-state.js',s=>ts.isVariableStatement(s)&&s.declarationList.declarations.some(d=>/^(beatCam|cinema|rtBeat|liveCam)/.test(d.name.getText()))));
const defaults=JSON.parse(fs.readFileSync(path.join(checkout,'public/default-user-fx-archive.json'),'utf8')).snapshot;
fs.writeFileSync(path.join(destination,'defaults.js'),header+'var fxDefaults='+JSON.stringify(defaults)+';\nObject.assign(fx,fxDefaults);\n');
fs.writeFileSync(path.join(destination,'float-layers.js'),header+declarations('02-visual/01-float-skull-backcover.js',s=>ts.isVariableStatement(s)?s.declarationList.declarations.some(d=>/^(FLOAT|float|BACK_COVER|backCover)/.test(d.name.getText())):ts.isFunctionDeclaration(s)&&['createFloatLayer','destroyFloatLayer','createBackCoverLayer','destroyBackCoverLayer','refreshBackCoverColorsFromCanvas','updateFloatLayer','refreshFloatColorsFromCover','resetFloatColorsToIdle'].includes(s.name.text)));
selected('07-fx/04-preset-grid-uniforms.js',['syncFxUniforms'],'uniforms.js');
selected('06-lyrics/00-lyrics-fetch-parse.js',['normalizeLyricTranslationText','isNoLyricText'],'lyric-text.js');
selected('04-shelf/00-layout-hover.js',['shelfLayoutProfile','isPortraitShelfViewport'],'shelf-layout.js');
selected('04-shelf/01-manager-core.js',['makeShelfManager'],'shelf-manager.js');
selected('02-visual/04-visual-settings-persistence.js',['normalizeShelfCameraMode','shelfDefaultAngleForCameraMode','normalizeShelfPresence','normalizedShelfNumber','shelfDetailSettings','shelfSummonSettings','durationEaseFactor','shelfSettings','shelfAlwaysVisible','shouldUseShelfDynamicCamera','shelfAccentHex','shelfAccentRgba'],'shelf-settings.js');

execFileSync('python3',[path.join(__dirname,'console-schema.py'),checkout],{stdio:'inherit'});

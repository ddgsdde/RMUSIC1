const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const root=path.join(__dirname,'../Sources/RMusicVisualAssets/Mineradio');
const THREE=require(path.join(root,'three.r128.min.js'));
function fixture(){
 const c=vm.createContext({THREE,performance:{now:()=>100}});vm.runInContext(fs.readFileSync(path.join(root,'lyric-viewport-fit.js'),'utf8'),c);
 const lyrics=new THREE.Group(),mesh=new THREE.Group(),rows=[];lyrics.add(mesh);
 for(let i=0;i<10;i++){
  const plane=new THREE.Mesh(new THREE.PlaneGeometry(8,1),new THREE.MeshBasicMaterial({transparent:true,opacity:.7}));
  plane.position.set(0,(i-4.5)*.48,0);mesh.add(plane);
  rows.push({mesh:plane,mat:plane.material,targetAlpha:1,lineWorldW:8,lineWorldH:1,lineMask:{inkBounds:{left:.04,right:.96,top:.15,bottom:.85}}});
 }
 mesh.userData.lyric={rowLayers:rows};return {fit:c.rmusicLyricViewportFit(),lyrics,rows};
}
test('all bilingual glyph corners fit after zoom, tilt and window height changes, without moving scene objects',()=>{
 const {fit,lyrics,rows}=fixture();const camera=new THREE.PerspectiveCamera(45,1,.01,100);
 lyrics.rotation.set(.12,.25,.06);
 for(const height of [396,488,650])for(const distance of [2.2,3,5]){
  const rect={width:1200,height};camera.position.set(0,.25,distance);camera.aspect=rect.width/rect.height;camera.updateProjectionMatrix();
  fit(camera,lyrics,rect,100+height+distance);
  for(const row of rows)for(const x of [-.46,.46])for(const y of [-.35,.35]){
   const p=new THREE.Vector3(x*8,y,0).applyMatrix4(row.mesh.matrixWorld).project(camera);
   assert(Math.abs(p.x)<=.921,`horizontal glyph clipped: ${p.x}`);assert(Math.abs(p.y)<.88,`vertical glyph clipped: ${p.y}`);
  }
 }
 assert.equal(lyrics.rotation.x,.12);assert.equal(lyrics.rotation.y,.25);assert.equal(rows[0].mesh.position.y,-2.16);
});

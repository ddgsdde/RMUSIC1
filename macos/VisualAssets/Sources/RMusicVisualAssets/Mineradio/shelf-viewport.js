// Keep the focused 3D card legible as the stage camera rotates or the window narrows.
function rmusicShelfViewportFit() {
  const points=Array.from({length:4},()=>new THREE.Vector3()),center=new THREE.Vector3(),target=new THREE.Vector3();
  return function(card,camera){
    if(!card?.mesh?.parent)return;
    const mesh=card.mesh,group=mesh.parent;
    group.scale.setScalar(1);group.updateMatrixWorld(true);camera.updateMatrixWorld(true);
    const w=mesh.geometry.parameters.width/2,h=mesh.geometry.parameters.height/2;
    function bounds(){
      let lo=Infinity,hi=-Infinity;
      [[-w,-h],[w,-h],[w,h],[-w,h]].forEach((p,i)=>{points[i].set(p[0],p[1],0).applyMatrix4(mesh.matrixWorld).project(camera);lo=Math.min(lo,points[i].x);hi=Math.max(hi,points[i].x);});
      return {lo,hi};
    }
    let b=bounds();const width=b.hi-b.lo;
    if(!Number.isFinite(width)||width<=0)return;
    if(width>.78){group.scale.setScalar(.78/width);group.updateMatrixWorld(true);b=bounds();}
    for(let pass=0;pass<3;pass++){
      const dx=b.hi>.935?.935-b.hi:b.lo<.165?.165-b.lo:0;
      if(!dx)break;
      mesh.getWorldPosition(center);center.project(camera);target.copy(center);target.x+=dx;target.unproject(camera);center.unproject(camera);group.position.add(target.sub(center));group.updateMatrixWorld(true);b=bounds();
    }
  };
}

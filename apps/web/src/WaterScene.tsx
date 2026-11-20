import {useEffect,useRef,useState} from "react";
import type {CoverPalette} from "./coverPalette";
// Low-resolution, dithered colour field. No readback or audio work in the frame loop.
const fragment=`precision highp float;
uniform vec2 resolution;uniform float time;uniform vec3 base;uniform vec3 light;uniform vec3 shade;
void main(){vec2 p=gl_FragCoord.xy/resolution;float t=time*.11;
float wave=sin(p.x*4.2+p.y*2.8+t)+.55*sin(p.y*6.1-p.x*2.2-t*.7);
float ribbon=sin((p.y+.16*sin(p.x*3.6+t*.8))*7.0+t*.55);
float a=smoothstep(-1.1,1.2,wave);vec3 c=mix(shade,base,a);c=mix(c,light,pow(.5+.5*ribbon,3.0)*.64);
float ripple=sin(length((p-vec2(.3+.08*sin(t),.6)))*15.0-t)*.012;c+=ripple;
float noise=fract(sin(dot(gl_FragCoord.xy,vec2(12.9898,78.233)))*43758.5453)-.5;
gl_FragColor=vec4(c+noise/255.0,1.0);}`;
export function WaterScene({palette,quiet}:{palette:CoverPalette;quiet:boolean}){
 const canvas=useRef<HTMLCanvasElement>(null),colours=useRef(palette);colours.current=palette;const[reduced,setReduced]=useState(()=>matchMedia("(prefers-reduced-motion: reduce)").matches);
 useEffect(()=>{const media=matchMedia("(prefers-reduced-motion: reduce)");const change=()=>setReduced(media.matches);media.addEventListener("change",change);return()=>media.removeEventListener("change",change);},[]);
 useEffect(()=>{const element=canvas.current;if(!element||quiet)return;const gl=element.getContext("webgl",{alpha:false,antialias:false,depth:false,powerPreference:"low-power"});if(!gl)return;
 const program=gl.createProgram()!;const shaders=[gl.createShader(gl.VERTEX_SHADER)!,gl.createShader(gl.FRAGMENT_SHADER)!];gl.shaderSource(shaders[0],"attribute vec2 point;void main(){gl_Position=vec4(point,0.,1.);}");gl.shaderSource(shaders[1],fragment);for(const shader of shaders){gl.compileShader(shader);if(!gl.getShaderParameter(shader,gl.COMPILE_STATUS)){for(const s of shaders)gl.deleteShader(s);gl.deleteProgram(program);return;}gl.attachShader(program,shader);}gl.linkProgram(program);if(!gl.getProgramParameter(program,gl.LINK_STATUS)){for(const s of shaders)gl.deleteShader(s);gl.deleteProgram(program);return;}
 gl.useProgram(program);const buffer=gl.createBuffer();gl.bindBuffer(gl.ARRAY_BUFFER,buffer);gl.bufferData(gl.ARRAY_BUFFER,new Float32Array([-1,-1,1,-1,-1,1,-1,1,1,-1,1,1]),gl.STATIC_DRAW);const point=gl.getAttribLocation(program,"point");gl.enableVertexAttribArray(point);gl.vertexAttribPointer(point,2,gl.FLOAT,false,0,0);const uniforms=Object.fromEntries(["resolution","time","base","light","shade"].map(k=>[k,gl.getUniformLocation(program,k)]));let frame=0,last=0,start=performance.now(),disposed=false;
 const size=()=>{const r=element.getBoundingClientRect(),scale=Math.min(1,720/Math.max(1,r.width),480/Math.max(1,r.height));element.width=Math.max(1,Math.round(r.width*scale));element.height=Math.max(1,Math.round(r.height*scale));gl.viewport(0,0,element.width,element.height);last=0;};
 const draw=(now:number)=>{if(disposed||document.hidden)return;if(now-last>=1000/24||last===0){last=now;gl.uniform2f(uniforms.resolution,element.width,element.height);gl.uniform1f(uniforms.time,(now-start)/1000);gl.uniform3fv(uniforms.base,colours.current.base);gl.uniform3fv(uniforms.light,colours.current.light);gl.uniform3fv(uniforms.shade,colours.current.shade);gl.drawArrays(gl.TRIANGLES,0,6);}if(!reduced)frame=requestAnimationFrame(draw);};
 const visible=()=>{cancelAnimationFrame(frame);if(!document.hidden){last=0;frame=requestAnimationFrame(draw);}};const resize=new ResizeObserver(()=>{size();visible();});resize.observe(element);size();visible();document.addEventListener("visibilitychange",visible);const lost=(event:Event)=>{event.preventDefault();cancelAnimationFrame(frame);disposed=true;};element.addEventListener("webglcontextlost",lost);
 return()=>{disposed=true;cancelAnimationFrame(frame);resize.disconnect();document.removeEventListener("visibilitychange",visible);element.removeEventListener("webglcontextlost",lost);gl.deleteBuffer(buffer);for(const shader of shaders)gl.deleteShader(shader);gl.deleteProgram(program);};
 },[quiet,reduced,palette]);
 return <div className="scene-water" aria-hidden="true" style={{background:`rgb(${palette.base.map(n=>Math.round(n*255)).join(",")})`}}>{!quiet&&<canvas ref={canvas}/>}</div>;
}

#!/usr/bin/env python3
"""Pinned visual algorithms -> native MSL at development time. No JS in the renderer."""
from pathlib import Path
import re,json,subprocess,tempfile,ast
ROOT=Path(__file__).resolve().parents[1]
SRC=ROOT/'VisualAssets/Sources/RMusicVisualAssets/Mineradio'
OUT=ROOT/'Sources/MineradioNative/Resources'

def template(text,name):
 m=re.search(r'var\s+'+re.escape(name)+r'\s*=\s*`([^`]+)`',text,re.S)
 if not m: raise ValueError(name)
 return m.group(1)

def string_array(source, pattern):
 m=re.search(pattern+r"\s*(\[.*?\])\.join\('\\n'\)",source,re.S)
 if not m: raise ValueError(pattern)
 value=re.sub(r"'\s*\+\s*RIPPLE_MAX\s*\+\s*'",'10',m[1])
 return '\n'.join(ast.literal_eval(value))

def compile_shelf():
 # Native textured planes preserve the pinned shelf geometry and canvas material.
 compile_program('shelf', '''
 attribute vec2 uv;
 varying vec2 vUv;
 void main(){vUv=uv;gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.0);}
 ''','''
 uniform sampler2D uMap;
 uniform float uOpacity;
 varying vec2 vUv;
 void main(){gl_FragColor=texture2D(uMap,vUv)*uOpacity;}
 ''')

def compile_sonic_particle():
 # sonic-topography.js buildSimpleInstanced: unlit, alpha blended, no tone mapping.
 compile_program('sonicParticle','''
 attribute mat4 instanceMatrix;
 void main(){gl_Position=projectionMatrix*modelViewMatrix*instanceMatrix*vec4(position,1.0);}
 ''','''
 uniform vec3 uColor;
 uniform float uOpacity;
 void main(){gl_FragColor=vec4(uColor,uOpacity);}
 ''')

def write_defaults():
 text=(SRC/'stage/defaults.js').read_text()
 value=json.loads(re.search(r'var fxDefaults=(\{.*\});',text).group(1))
 (OUT/'defaults.json').write_text(json.dumps(value,ensure_ascii=False,indent=2)+'\n')

def antialias_terrain_rims(fragment):
 # MSAA covers geometry silhouettes, not the very narrow procedural highlight
 # inside each face. Widen only subpixel transitions by their screen footprint.
 changes={
  'float edgeX=smoothstep(0.05,0.01,vUv.x)+smoothstep(0.95,0.99,vUv.x);':
   'float aaX=max(0.02,fwidth(vUv.x)*0.5);\n'
   '    float edgeX=1.0-smoothstep(0.03-aaX,0.03+aaX,min(vUv.x,1.0-vUv.x));',
  'float edgeY=smoothstep(0.05,0.01,vUv.y)+smoothstep(0.95,0.99,vUv.y);':
   'float aaY=max(0.02,fwidth(vUv.y)*0.5);\n'
   '    float edgeY=1.0-smoothstep(0.03-aaY,0.03+aaY,min(vUv.y,1.0-vUv.y));',
  'float rimGlow=smoothstep(0.03,0.0,distFromTop)*normElevation;':
   'float aaRim=max(0.015,fwidth(distFromTop)*0.5);\n'
   '    float rimGlow=(1.0-smoothstep(0.015-aaRim,0.015+aaRim,distFromTop))*normElevation;'
 }
 for old,new in changes.items():
  if fragment.count(old)!=1: raise ValueError('Terrain rim source changed')
  fragment=fragment.replace(old,new)
 return fragment

def compile_lyric_glow():
 # Bounded nine-tap glow uses the resident CoreText mask, without a second bitmap cache.
 compile_program('lyricGlow','''
 attribute vec3 position; attribute vec2 uv; varying vec2 vUv;
 void main(){vUv=uv;gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.0);}
 ''','''
 uniform sampler2D uMap;
 uniform vec2 uRadius;
 uniform vec3 uColor;
 uniform float uOpacity;
 varying vec2 vUv;
 void main(){
   float a=texture2D(uMap,vUv).a*0.2;
   a+=(texture2D(uMap,vUv+vec2(uRadius.x,0.0)).a+texture2D(uMap,vUv-vec2(uRadius.x,0.0)).a+texture2D(uMap,vUv+vec2(0.0,uRadius.y)).a+texture2D(uMap,vUv-vec2(0.0,uRadius.y)).a)*0.12;
   a+=(texture2D(uMap,vUv+uRadius).a+texture2D(uMap,vUv-uRadius).a+texture2D(uMap,vUv+vec2(uRadius.x,-uRadius.y)).a+texture2D(uMap,vUv+vec2(-uRadius.x,uRadius.y)).a)*0.08;
   gl_FragColor=vec4(uColor,a*uOpacity);
 }
 ''')

def compile_program(name,vertex,fragment):
 stages={'vert':vertex,'frag':fragment};uniforms={};textures={};attributes={};varyings={}
 for stage,source in stages.items():
  for typ,names in re.findall(r'uniform\s+(\w+)\s+([^;]+);',source):
   for n in names.split(','):
    n=n.strip();(textures if typ.startswith('sampler') else uniforms)[n]=typ
  for typ,names in re.findall(r'attribute\s+(\w+)\s+([^;]+);',source):
   for n in names.split(','):attributes[n.strip()]=typ
  for typ,names in re.findall(r'varying\s+(\w+)\s+([^;]+);',source):
   for n in names.split(','):varyings[n.strip()]=typ
 uniforms.update(modelViewMatrix='mat4',projectionMatrix='mat4')
 for n,t in {'normalMatrix':'mat3','modelMatrix':'mat4','viewMatrix':'mat4'}.items():
  if re.search(r'\b'+n+r'\b',vertex):uniforms[n]=t
 for n,t in {'position':'vec3','uv':'vec2','normal':'vec3','instanceMatrix':'mat4'}.items():
  if n=='position' or re.search(r'\b'+n+r'\b',vertex):attributes.setdefault(n,t)
 locations={};location=0
 for n,t in attributes.items():locations[n]=location;location+=4 if t=='mat4' else 1
 structures={m[0]:m[1] for source in stages.values() for m in re.findall(r'(?:struct\s+(\w+)\s*(\{.*?\};))',source,re.S)}
 definitions=''.join('struct '+n+' '+body+'\n' for n,body in structures.items())
 block=definitions+'layout(std140,binding=0) uniform StageUniforms {\n'+''.join(f' {t} {n};\n' for n,t in uniforms.items())+'};\n'
 samplers=''.join(f'layout(binding={i+1}) uniform {t} {n};\n' for i,(n,t) in enumerate(textures.items()))
 manifest={'name':name,'textures':[{'name':n,'binding':i+1} for i,n in enumerate(textures)],'stages':{}}
 with tempfile.TemporaryDirectory(prefix='rmusic-native-shader-') as directory:
  folder=Path(directory)
  for stage,source in stages.items():
   source=re.sub(r'precision\s+\w+\s+\w+\s*;','',source)
   source=re.sub(r'uniform\s+\w+\s+[^;]+;','',source)
   source=re.sub(r'struct\s+\w+\s*\{.*?\};','',source,flags=re.S)
   source=re.sub(r'attribute\s+(\w+)\s+([^;]+);',lambda m:'\n'.join(f'layout(location={locations[n.strip()]}) in {m[1]} {n.strip()};' for n in m[2].split(',')),source)
   source=re.sub(r'varying\s+(\w+)\s+([^;]+);',lambda m:'\n'.join(f'layout(location={list(varyings).index(n.strip())}) {"out" if stage=="vert" else "in"} {m[1]} {n.strip()};' for n in m[2].split(',')),source)
   source=source.replace('texture2D(', 'texture(')
   prefix='#version 450\n'+block+samplers
   if stage=='vert':
    for n,t in attributes.items():
     if not re.search(r'\bin '+t+' '+n+r';',source):prefix+=f'layout(location={locations[n]}) in {t} {n};\n'
   else:prefix+='layout(location=0) out vec4 nativeColor;\n';source=source.replace('gl_FragColor','nativeColor')
   glsl=folder/f'{name}.{stage}';spv=folder/f'{name}.{stage}.spv';glsl.write_text(prefix+source)
   result=subprocess.run(['glslangValidator','-V','-S',stage,str(glsl),'-o',str(spv)],text=True,capture_output=True)
   if result.returncode:raise RuntimeError(name+' '+stage+'\n'+result.stdout+result.stderr)
   msl=subprocess.check_output(['spirv-cross',str(spv),'--msl','--msl-version','20300','--msl-decoration-binding','--rename-entry-point','main',name+'_'+stage,stage],text=True)
   msl='// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.\n'+msl
   (OUT/f'{name}.{stage}.metal').write_text(msl.rstrip()+'\n')
   reflection=json.loads(subprocess.check_output(['spirv-cross',str(spv),'--reflect'],text=True))
   manifest['stages'][stage]=reflection
 (OUT/f'{name}.json').write_text(json.dumps(manifest,indent=2))
 print(name, 'native MSL generated')

if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 write_defaults()
 compile_shelf()
 compile_sonic_particle()
 compile_lyric_glow()
 lyric_sparks=(SRC/'stage/13-lyrics-mesh-build.js').read_text().split('var pmat =',1)[1]
 compile_program('lyricSpark',string_array(lyric_sparks,r'vertexShader:'),string_array(lyric_sparks,r'fragmentShader:'))
 p=(SRC/'particles.js').read_text();vs=template(p,'vs')
 compile_program('cover',vs,template(p,'fs'))
 bloom=vs.replace('uniform float uMouseActive, uPixel, uColorMixT, uLoading;','uniform float uMouseActive, uPixel, uColorMixT, uLoading, uBloomSize;').replace('gl_PointSize = sz * uPixel * uPointScale;','gl_PointSize = sz * uPixel * uPointScale * uBloomSize;')
 compile_program('bloom',bloom,template(p,'bloomFs'))
 compile_program('stars',template(p,'backgroundStarRiverVs'),template(p,'backgroundStarRiverFs'))
 back=(SRC/'stage/float-layers.js').read_text().split('function createBackCoverLayer()',1)[1].split('function destroyBackCoverLayer()',1)[0]
 back_vertex=template(back,'vs').replace('attribute vec3 aColor;','uniform sampler2D uCoverTex;').replace('vC = aColor;','vC = texture2D(uCoverTex,aUv).rgb * 0.85;')
 compile_program('backCover',back_vertex,template(back,'fs'))

 skull=(SRC/'skull.js').read_text()
 compile_program('skull',string_array(skull,r'vertexShader:'),string_array(skull,r'fragmentShader:'))
 lyric=(SRC/'stage/11-lyrics-shaders.js').read_text()
 lv=ast.literal_eval(re.search(r"vertexShader:\s*('(?:[^'\\]|\\.)*')",lyric).group(1))
 compile_program('lyrics',lv,string_array(lyric,r'fragmentShader:'))
 terrain=(SRC/'sonic-topography.js').read_text()
 tv=string_array(terrain,r'function buildTerrainVertexShader\(\)\s*\{\s*return')
 tf=antialias_terrain_rims(string_array(terrain,r'function buildTerrainFragmentShader\(\)\s*\{\s*return'))
 compile_program('terrain',tv,tf)
 compile_program('floating',string_array(terrain,r'function buildFloatingVertexShader\(\)\s*\{\s*return'),tf)
 workshop=(SRC/'workshop/assets/index-Z-j1MQ-r.js').read_text()
 strings=re.findall(r'`([^`]+)`',workshop,re.S)
 compile_program('workshop',next(x for x in strings if 'gl_Position' in x and 'uAudioIntensity' in x),next(x for x in strings if 'gl_FragColor' in x and 'uWarmth' in x))

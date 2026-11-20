# Pinned Mineradio control metadata (GPL-3.0); invoked by import-mineradio.cjs.
from pathlib import Path
import sys
import re,json,html
src=Path(sys.argv[1])/'public'
root=Path(__file__).resolve().parents[1]/'Sources/RMusicVisualAssets/Mineradio'
defaults=json.loads((src/'default-user-fx-archive.json').read_text())['snapshot']
bindings=(src/'js/modules/07-fx/07-bindings-shelf-immersive.js').read_text()
keys=dict(re.findall(r"\['(fx-[^']+)', '(\w+)'\]",bindings))
inputs={m.group(1):dict(re.findall(r'([\w-]+)="([^"]*)"',m.group(0))) for m in re.finditer(r'<input\b[^>]*\bid="([^"]+)"[^>]*>',(src/'index.html').read_text())}
layout=(src/'js/modules/07-fx/09-console-workspace.js').read_text()
labels=dict(re.findall(r"fxConsoleItem\('([^']+)', '([^']+)'",layout))
for key,label,low,high,step in re.findall(r"ensureFxRangeControl\('[^']+', '([^']+)', '([^']+)', ([\d.-]+), ([\d.-]+), ([\d.-]+)\)", (src/'js/modules/07-fx/05-fx-panel-performance.js').read_text()):
 inputs[key]=dict(min=low,max=high,step=step)
fields=[]
def ranges(tab,group,names):
 for key in names.split():
  ref=next((ref for ref,value in keys.items() if value==key),None)
  if not ref or ref not in inputs: raise Exception((key,ref))
  attrs=inputs[ref]
  fields.append(dict(id=key,tab=tab,group=group,label=labels.get(ref,key),kind='range',number=defaults[key],minimum=float(attrs['min']),maximum=float(attrs['max']),step=float(attrs.get('step',1))))
def toggles(tab,group,names):
 for key in names.split():fields.append(dict(id=key,tab=tab,group=group,label=labels.get('t-'+key,key),kind='toggle',flag=defaults[key]))
def choice(tab,group,key,label,options):fields.append(dict(id=key,tab=tab,group=group,label=label,kind='choice',text=defaults[key],choices=[dict(id=x.split(':')[0],label=x.split(':')[1]) for x in options.split()]))
ranges('动效','基础画面','depth cinemaShake')
toggles('动效','基础画面','cinema')
ranges('动效','粒子与光影','point speed twist color bloomStrength scatter bgFade')
toggles('动效','粒子与光影','bloom edge backgroundStarRiver')
ranges('歌词','显示与翻译','lyricCustomLineCount lyricTranslationGap lyricTranslationScale lyricTranslationOpacity')
choice('歌词','显示与翻译','lyricDisplayMode','歌词行数','single:单行 dual:双行 triple:三行 cinema:沉浸 custom:自定义')
choice('歌词','显示与翻译','lyricTranslationMode','双语翻译','off:关闭 current:当前行 multi:多行')
ranges('歌词','字体与排版','lyricLetterSpacing lyricLineHeight lyricWeight lyricScale lyricOffsetX lyricOffsetY lyricOffsetZ lyricTiltX lyricTiltY')
choice('歌词','字体与排版','lyricFont','歌词字体','sans:黑体 song:宋体 mono:等宽')
ranges('歌词','颜色与光效','lyricGlowStrength lyricBackgroundAdapt')
toggles('歌词','颜色与光效','lyricGlow lyricGlowBeat lyricGlowParticles')
ranges('歌词','歌词动画','lyricContextOpacity lyricContextSpread lyricEdgeFade lyricMotionSoftness')
toggles('歌词','歌词动画','lyricVerticalFloat lyricCameraLock lyricPauseHold')
choice('歌词','歌词动画','lyricMotionStyle','歌词动画','float:漂浮 smooth:柔滑 glass:玻璃 shine:线光 glitch:故障')
ranges('歌单架','外观与位置','shelfSize shelfOffsetX shelfOffsetY shelfOffsetZ shelfAngleY shelfOpacity shelfBgOpacity')
choice('歌单架','显示方式','shelfCameraMode','镜头','dynamic:动态 static:静态')
ranges('动效','音域地形','sonicGroundAmplitude sonicGroundMotionSpeed sonicGroundRange sonicGroundLower sonicGroundDepth sonicGroundAutoRotate sonicGroundGlow')
# Tint affects only the GPU scene, not the preserved native theme palettes.
choice('界面','视觉配色','visualTintMode','视觉主色','auto:封面取色 custom:自定义')
fields.append(dict(id='visualTintColor',tab='界面',group='视觉配色',label='自定义颜色',kind='color',text=defaults['visualTintColor']))
(root/'console.json').write_text(json.dumps(fields,ensure_ascii=False,indent=2)+'\n')
(root/'stage/console-schema.js').write_text('// Pinned Mineradio control bounds; native and GPU share one schema. GPL-3.0.\nvar visualControlSchema='+json.dumps(fields,ensure_ascii=False)+';\n')
print(len(fields),'supported visual controls')

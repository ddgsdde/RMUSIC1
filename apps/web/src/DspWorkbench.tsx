import {useLayoutEffect,useMemo,useRef,useState} from "react";
import type {PointerEvent} from "react";
import {Activity, AudioLines, Cable, ChartSpline, Gauge, Plus, RotateCcw, SlidersHorizontal, Trash2, Volume2, Waves} from "lucide-react";
import {normalizeDsp,type DspBand,type DspConfig,type DspEqStage,type Endpoint,type EqFilter,type RMusicClient,type Zone} from "@r-music/sdk";
import {ImpulseLibrary} from "./ImpulseLibrary";
import {bandResponse,cutFilter,displayFrequency,filterNames,gainFilter} from "./dspResponse";
import {equalizers,installedModules,maxChainBands,maxEqualizers,withEqualizers,type DspSlot} from "./dspModules";
import "./DspWorkbench.css";

const rates=[8000,11025,16000,22050,24000,32000,44100,48000,88200,96000,176400,192000,352800,384000,705600,768000];
const colors=["#64dcb8","#a99af8","#ebb977","#73c1f4","#f498a7","#c0da78"];
const moduleList=[
  ["rate","采样率转换",Activity],["stereo","立体声",AudioLines],["loudness","响度均衡",Volume2],
  ["gain","增益与余量",Gauge],["eq","参数均衡",ChartSpline],["fades","淡入淡出",Waves],["convolution","卷积",Cable],
] as const;
type Module=typeof moduleList[number][0];
type EditorProps={api:RMusicClient;zone:Zone;capabilities?:Endpoint["capabilities"];advanced?:boolean;modular?:boolean;onSaved:()=>void;onError:(error:unknown)=>void};
const palettes=[["graphite","石墨"],["ink","蓝墨"],["plum","暮紫"],["paper","暖白"]] as const;
type Palette=typeof palettes[number][0];
function savedPalette():Palette {
  try {const saved=localStorage.getItem("rmusic.dsp.palette");return palettes.find(([id])=>id===saved)?.[0]??"graphite";}
  catch{return "graphite";}
}
const defaults={crossfeed:0,crossover_hz:700,mono:false,invert_left:false,invert_right:false};
const clamp=(v:number,min:number,max:number)=>Math.max(min,Math.min(max,v));

function EqPlot({bands,rate,bypassed,selected,select,change,add,palette}:{palette:Palette;bands:DspBand[];rate:number;bypassed:boolean;selected:number;select:(i:number)=>void;change:(i:number,values:Partial<DspBand>)=>void;add:(hz:number,db:number)=>void}) {
  const canvas=useRef<HTMLCanvasElement>(null), drag=useRef(-1);
  const [size,setSize]=useState({width:0,height:0}),[dbRange,setDbRange]=useState(18);
  const high=Math.max(1000,Math.min(20000,rate/2*0.99));
  const inset={x:49,y:15,right:19,bottom:33};
  const width=Math.max(1,size.width-inset.x-inset.right),height=Math.max(1,size.height-inset.y-inset.bottom);
  const x=(hz:number)=>inset.x+clamp(Math.log(hz/10)/Math.log(high/10),0,1)*width;
  const y=(db:number)=>inset.y+(dbRange-clamp(db,-dbRange,dbRange))*height/(2*dbRange);
  const points=useMemo(()=>Array.from({length:241},(_,i)=>10*(high/10)**(i/240)),[high]);
  const active=bands[selected];
  const curves=useMemo(()=>({
    combined:bands.reduce((curve,band)=>bypassed?curve:bandResponse(band,rate,points).map((db,i)=>curve[i]+db),points.map(()=>0)),
    individual:active?bandResponse({...active,enabled:true},rate,points):null,
  }),[bands,rate,points,bypassed,active]);
  useLayoutEffect(()=>{
    const node=canvas.current;if(!node||typeof ResizeObserver==="undefined")return;
    const observer=new ResizeObserver(([entry])=>setSize({width:entry.contentRect.width,height:entry.contentRect.height}));
    observer.observe(node);return()=>observer.disconnect();
  },[]);
  useLayoutEffect(()=>{
    const node=canvas.current;if(!node||size.width<1||size.height<1)return;
    const dpr=window.devicePixelRatio||1; node.width=Math.round(size.width*dpr);node.height=Math.round(size.height*dpr);
    const ctx=node.getContext("2d");if(!ctx)return;
    const style=getComputedStyle(node);
    const tone=(name:string,fallback:string)=>style.getPropertyValue("--eq-"+name).trim()||fallback;
    ctx.scale(dpr,dpr);ctx.font="11px ui-monospace, monospace";ctx.textBaseline="middle";
    const line=(x1:number,y1:number,x2:number,y2:number,stroke:string)=>{ctx.strokeStyle=stroke;ctx.beginPath();ctx.moveTo(x1,y1);ctx.lineTo(x2,y2);ctx.stroke();};
    for(const db of [-dbRange,-dbRange/2,0,dbRange/2,dbRange]){
      line(inset.x,y(db),size.width-inset.right,y(db),db===0?tone("axis","#687b7b"):tone("grid","#273536"));
      ctx.fillStyle=tone("text","#9faead");ctx.fillText(db===0?"0 dB":`${db>0?"+":""}${db}`,8,y(db));
    }
    ctx.textAlign="center";
    for(const hz of [10,20,50,100,200,500,1000,2000,5000,10000,20000].filter(hz=>hz<=high)){
      line(x(hz),inset.y,x(hz),y(-dbRange),tone("grid","#283737"));ctx.fillStyle=tone("text","#a6b5b2");
      if(size.width>500||[10,50,200,1000,5000,20000].includes(hz))ctx.fillText(hz>=1000?`${hz/1000}k`:`${hz}`,x(hz),size.height-13);
    }
    const trace=(values:number[])=>{ctx.beginPath();values.forEach((db,i)=>i===0?ctx.moveTo(x(points[i]),y(db)):ctx.lineTo(x(points[i]),y(db)));};
    trace(curves.combined);ctx.lineTo(x(high),y(0));ctx.lineTo(x(10),y(0));ctx.closePath();
    const fill=ctx.createLinearGradient(0,inset.y,0,y(-dbRange));fill.addColorStop(0,tone("fill","#55d8ac55"));fill.addColorStop(1,"transparent");ctx.fillStyle=fill;ctx.fill();
    if(curves.individual){trace(curves.individual);ctx.strokeStyle=colors[selected%colors.length];ctx.globalAlpha=.55;ctx.setLineDash([4,5]);ctx.stroke();ctx.setLineDash([]);ctx.globalAlpha=1;}
    trace(curves.combined);ctx.strokeStyle=bypassed?tone("text","#9eaeac"):tone("curve","#80edcd");ctx.lineWidth=2;ctx.stroke();
    bands.forEach((b,i)=>{
      if(b.frequency>high)return;
      const picked=i===selected, cx=x(b.frequency),cy=y(gainFilter(b.kind)?b.gain_db:0);
      ctx.globalAlpha=b.enabled?1:.5;ctx.beginPath();ctx.arc(cx,cy,picked?12:9,0,2*Math.PI);
      ctx.fillStyle=picked?colors[i%colors.length]:"#162423";ctx.strokeStyle=colors[i%colors.length];ctx.fill();ctx.lineWidth=1.5;ctx.stroke();
      ctx.fillStyle=picked?"#10231e":"#e0ebe7";ctx.fillText(String(i+1),cx,cy);ctx.globalAlpha=1;
    });
  });
  const point=(event:PointerEvent<HTMLCanvasElement>)=>{
    const bounds=event.currentTarget.getBoundingClientRect(),px=event.clientX-bounds.left,py=event.clientY-bounds.top;
    return {px,py,hz:Math.round(10*(high/10)**clamp((px-inset.x)/width,0,1)),db:Math.round(clamp(dbRange-(py-inset.y)*2*dbRange/height,-30,30)*10)/10};
  };
  return <div className="eq-display"><div className="eq-display-head"><span>综合响应 <small>· {displayFrequency(rate)} 预览</small></span><select aria-label="图表增益刻度" value={dbRange} onChange={e=>setDbRange(Number(e.target.value))}>{[6,12,18,30].map(db=><option key={db} value={db}>±{db} dB</option>)}</select></div>
    <canvas ref={canvas} data-color={palette} aria-label="均衡器响应图" onPointerDown={event=>{
      if(event.button!==0)return;const {px,py}=point(event);
      const candidates=bands.map((band,i)=>({i,d:band.frequency>high?Infinity:Math.hypot(px-x(band.frequency),py-y(gainFilter(band.kind)?band.gain_db:0))})).sort((a,b)=>a.d-b.d);
      if(candidates[0]?.d>18||!candidates.length)return;
      drag.current=candidates[0].i;select(drag.current);event.currentTarget.setPointerCapture(event.pointerId);
    }} onPointerMove={event=>{const i=drag.current,band=bands[i];if(!band)return;const {hz,db}=point(event);change(i,{frequency:hz,...(gainFilter(band.kind)?{gain_db:db}:{})});}}
    onLostPointerCapture={()=>{drag.current=-1;}} onPointerUp={()=>{drag.current=-1;}} onPointerCancel={()=>{drag.current=-1;}}
    onDoubleClick={event=>{
      const bounds=event.currentTarget.getBoundingClientRect(),px=event.clientX-bounds.left,py=event.clientY-bounds.top;
      if(bands.some(b=>Math.hypot(px-x(b.frequency),py-y(gainFilter(b.kind)?b.gain_db:0))<=18))return;
      add(Math.round(10*(high/10)**clamp((px-inset.x)/width,0,1)),Math.round(clamp(dbRange-(py-inset.y)*2*dbRange/height,-30,30)*10)/10);
    }}/>
    <div className="eq-display-foot">拖动频段调节频率和增益，双击空白添加频段。</div>
  </div>;
}

export function DspEditor({api,zone,onSaved,onError,capabilities,advanced=false,modular=false}:EditorProps){
  const portable=zone.id.startsWith("client-zone-")||zone.id.startsWith("travel-");
  const core=capabilities?.core_pcm_dsp_v1===true, pro=advanced&&core;
  const supported=capabilities?.dsp!==false, transitionsSupported=capabilities?.transitions!==false;
  const convolution=core||capabilities?.convolution_v1===true;
  const [slot,setSlot]=useState<DspSlot>({kind:portable?"loudness":"eq",index:0});
  const [palette,setPalette]=useState(savedPalette);
  const baseline=()=>normalizeDsp(zone.dsp);
  const [config,setConfig]=useState(baseline),[fades,setFades]=useState(()=>({fade_in_ms:zone.transitions?.fade_in_ms??0,fade_out_ms:zone.transitions?.fade_out_ms??0}));
  const [savedConfig,setSavedConfig]=useState(baseline),[savedFades,setSavedFades]=useState(fades);
  const [bandIndex,select]=useState(0),[saving,setSaving]=useState(false),[status,setStatus]=useState(""),[importing,setImporting]=useState(false),[analyzing,setAnalyzing]=useState(false);
  const presetInput=useRef<HTMLInputElement>(null);
  const dirtyDsp=JSON.stringify(config)!==JSON.stringify(savedConfig),dirtyFade=JSON.stringify(fades)!==JSON.stringify(savedFades);
  const dirty=dirtyDsp||dirtyFade;
  const stages=equalizers(config), installed=installedModules(config,fades,portable);
  const allowed=moduleList.filter(([key])=>portable?key==="loudness"||key==="fades":(key!=="rate"||core)&&(key!=="stereo"||pro));
  const repeated=modular&&core;
  const slots=allowed.flatMap(([kind])=>installed.has(kind)?Array.from({length:kind==="eq"&&repeated?stages.length:1},(_,index)=>({kind,index})):[]);
  const selectedSlot=slots.find(entry=>entry.kind===slot.kind&&entry.index===slot.index)??slots[0];
  const module=selectedSlot?.kind, eqIndex=module==="eq"?selectedSlot.index:0;
  const eq=stages[eqIndex], band=eq.bands[bandIndex];
  const previewRate=Math.max(8000,config.sample_rate??(zone.track?.sample_rate||48000));
  const stereo=config.stereo??defaults;
  const update=(change:Partial<DspConfig>)=>setConfig(previous=>({...previous,...change}));
  const writeEq=(current:DspConfig,change:(stage:DspEqStage)=>DspEqStage)=>{
    const entries=equalizers(current);entries[eqIndex]=change(entries[eqIndex]);return withEqualizers(current,entries);
  };
  const changeBand=(index:number,values:Partial<DspBand>)=>setConfig(current=>writeEq(current,stage=>({...stage,bands:stage.bands.map((entry,i)=>i===index?{...entry,...values}:entry)})));
  const totalBands=stages.reduce((sum,stage)=>sum+stage.bands.length,0);
  const addBand=(hz=1000,db=0)=>{
    if(eq.bands.length>=32||totalBands>=maxChainBands)return;
    select(eq.bands.length);
    setConfig(current=>writeEq({...current,enabled:true},stage=>({...stage,bands:[...stage.bands,{enabled:true,frequency:hz,gain_db:db,q:1}]})));
  };
  const removeBand=()=>{select(Math.max(0,bandIndex-1));setConfig(current=>writeEq(current,stage=>({...stage,bands:stage.bands.filter((_,i)=>i!==bandIndex)})));};
  function addModule(kind:Module){
    if(installed.has(kind)){
      if(kind!=="eq"||!repeated||stages.length>=maxEqualizers)return;
      setConfig(current=>withEqualizers(current,[...equalizers(current),{bypass:false,bands:[]}]));
      setSlot({kind,index:stages.length});
    }else{
      update({modules:[...installed,kind]});
      setSlot({kind,index:0});
    }
    select(0);
  }
  function removeModule(){
    if(!module)return;
    if(module==="eq"&&stages.length>1){
      setConfig(current=>withEqualizers(current,equalizers(current).filter((_,i)=>i!==eqIndex)));
      setSlot({kind:"eq",index:Math.max(0,eqIndex-1)});
    }else{
      const remaining=[...installed].filter(kind=>kind!==module);
      const reset:Record<Module,Partial<DspConfig>>={eq:{bands:[],eq_bypass:undefined,extra_eq:undefined},gain:{preamp_db:0,balance:0},rate:{sample_rate:null},stereo:{stereo:null},loudness:{normalize:false,target_lufs:-14},convolution:{convolution:null},fades:{}};
      update({...reset[module],modules:remaining});
      if(module==="fades")setFades({fade_in_ms:0,fade_out_ms:0});
    }
    select(0);
  }
  function moveEqualizer(step:number){
    const next=eqIndex+step;if(next<0||next>=stages.length)return;
    const entries=stages.slice();[entries[eqIndex],entries[next]]=[entries[next],entries[eqIndex]];
    setConfig(current=>withEqualizers(current,entries));setSlot({kind:"eq",index:next});
  }
  const path=`/zones/${encodeURIComponent(zone.id)}`;
  const eqSweep=Array.from({length:200},(_,i)=>10*(Math.min(20000,previewRate/2*.99)/10)**(i/199));
  const allBands=stages.filter(stage=>!stage.bypass).flatMap(stage=>stage.bands);
  const eqPeak=module==="gain"?Math.max(0,...allBands.reduce((curve,b)=>bandResponse(b,previewRate,eqSweep).map((db,i)=>curve[i]+db),eqSweep.map(()=>0))):0;
  const descriptions:Record<Module,string>={eq:"",gain:`${config.preamp_db.toFixed(1)} dB`,rate:config.sample_rate?displayFrequency(config.sample_rate):"跟随音源",stereo:config.stereo?"已配置":"原始声道",convolution:config.convolution?`${config.convolution.response_ids.length} 组响应`:"未选响应",loudness:config.normalize?`${config.target_lufs} LUFS`:"关闭",fades:`${fades.fade_in_ms} / ${fades.fade_out_ms} ms`};
  const enabled:Record<Module,boolean>={eq:false,gain:config.enabled&&(config.preamp_db!==0||config.balance!==0),rate:config.enabled&&!!config.sample_rate,stereo:config.enabled&&!!config.stereo,convolution:config.enabled&&!!config.convolution,loudness:(config.enabled||portable)&&config.normalize,fades:!!(fades.fade_in_ms||fades.fade_out_ms)};
  const addable=allowed.filter(([kind])=>!installed.has(kind)||kind==="eq"&&repeated&&stages.length<maxEqualizers);
  async function save(){
    if(!dirty||saving)return;setSaving(true);setStatus("");
    try{
      if(dirtyDsp){await api.put(path+"/dsp",config);setSavedConfig(config);}
      if(dirtyFade){await api.patch(path,{transitions:fades});setSavedFades(fades);}
      setStatus("已应用到此音区");onSaved();
    }catch(error){setStatus("保存失败，草稿已保留");onError(error);}finally{setSaving(false);}
  }
  return <form className="dsp-workspace" data-palette={palette} onSubmit={event=>{event.preventDefault();void save();}}>
    <div className="dsp-bar"><div><SlidersHorizontal size={21}/><div><strong>{zone.name}</strong><span>{portable?"本机外出播放":core?"Core 处理":"播放端处理"}</span></div></div>
      {!portable&&supported&&<label className="dsp-master"><input type="checkbox" checked={config.enabled} onChange={e=>update({enabled:e.target.checked})}/>启用 DSP</label>}
      <div className="dsp-actions"><select aria-label="DSP 配色" value={palette} onChange={e=>{const value=e.target.value as Palette;setPalette(value);try{localStorage.setItem("rmusic.dsp.palette",value);}catch{}}}>{palettes.map(([id,name])=><option key={id} value={id}>{name}</option>)}</select><span role="status">{dirty?"有未保存更改":status}</span><button type="button" className="dsp-reset" disabled={!dirty||saving} onClick={()=>{setConfig(savedConfig);setFades(savedFades);select(0);setStatus("");}}><RotateCcw size={15}/>还原</button><button className="primary" disabled={!dirty||saving}>{saving?"正在应用…":"应用到音区"}</button></div>
    </div>
    <div className="dsp-layout">
      <aside className="dsp-chain">
        <div className="dsp-add-module"><select aria-label="添加模块" value="" disabled={!addable.length||saving||importing} onChange={e=>addModule(e.target.value as Module)}>
          <option value="" disabled>＋ 添加模块</option>{addable.map(([kind,title])=><option key={kind} value={kind}>{title}</option>)}
        </select></div>
        <nav aria-label="DSP 模块" className="dsp-module-list">{slots.map(({kind,index})=>{
          const [,title,Icon]=moduleList.find(([id])=>id===kind)!;
          const rows=kind==="eq"?stages[index].bands:[], label=kind==="eq"&&stages.length>1?`${title} ${index+1}`:title;
          const live=kind==="eq"?config.enabled&&!stages[index].bypass&&rows.some(b=>b.enabled):enabled[kind];
          return <button type="button" key={`${kind}-${index}`} aria-current={module===kind&&selectedSlot?.index===index?"page":undefined} disabled={importing} onClick={()=>{setSlot({kind,index});select(0);}}><Icon size={18}/><span><strong>{label}</strong><small>{kind==="eq"?`${rows.filter(b=>b.enabled).length} 段`:descriptions[kind]}</small></span><i data-active={live} aria-label={live?"已启用":"已旁路"}/></button>;
        })}</nav>
      </aside>
      <section className="dsp-module" key={`${module}-${eqIndex}`}>
        {module?<div className="dsp-module-title"><h2>{moduleList.find(([id])=>id===module)?.[1]}{module==="eq"&&stages.length>1?` ${eqIndex+1}`:""}</h2><div>
          {module==="eq"&&supported&&<>
            <button type="button" disabled={importing} onClick={()=>presetInput.current?.click()}>{importing?"读取中…":"导入预设"}</button>
            <button type="button" disabled={eq.bands.length>=32||totalBands>=maxChainBands} onClick={()=>addBand()}><Plus size={16}/>添加频段</button>
            {stages.length>1&&<><button type="button" aria-label="上移 EQ" disabled={eqIndex===0||importing} onClick={()=>moveEqualizer(-1)}>↑</button><button type="button" aria-label="下移 EQ" disabled={eqIndex===stages.length-1||importing} onClick={()=>moveEqualizer(1)}>↓</button></>}
          </>}
          <button type="button" aria-label="移除模块" title="移除模块" disabled={saving||importing} onClick={removeModule}><Trash2 size={16}/></button>
        </div></div>:<p className="dsp-note">从「添加模块」开始配置音频处理。</p>}
        {!supported&&module!=="fades"&&<p className="dsp-note">此输出的 DSP 由设备管理。</p>}
        {module==="eq"&&supported&&<>
          <input ref={presetInput} type="file" hidden accept=".txt,.eq,.conf,.json,.csv" onChange={async e=>{
            const file=e.currentTarget.files?.[0];e.currentTarget.value="";if(!file)return;setImporting(true);
            try{if(file.size>65536)throw new Error("预设超过 64 KiB");const preset=await api.post<Pick<DspConfig,"bands"|"preamp_db">>("/dsp/equalizer/import",{text:await file.text()});
              const bands=pro?preset.bands:preset.bands.map(({slope_db_oct,...entry})=>entry);
              if(totalBands-eq.bands.length+bands.length>maxChainBands)throw Error("该音区的 EQ 频段合计最多 64 段");
              setConfig(current=>writeEq({...current,preamp_db:preset.preamp_db,enabled:true},()=>({bands,bypass:false})));select(0);
            }catch(error){onError(error);}finally{setImporting(false);}
          }}/>
          <EqPlot palette={palette} bands={eq.bands} rate={previewRate} selected={bandIndex} bypassed={eq.bypass} select={select} change={changeBand} add={addBand}/>
          {pro&&<label className="dsp-inline"><input type="checkbox" checked={eq.bypass} onChange={e=>{const bypass=e.target.checked;setConfig(current=>writeEq(current,stage=>({...stage,bypass})));}}/>旁路参数均衡</label>}
          {eq.bands.length>0&&<div className="eq-band-tabs" aria-label="频段">{eq.bands.map((item,i)=><button type="button" key={i} aria-pressed={bandIndex===i} onClick={()=>select(i)} style={{"--band":colors[i%colors.length]} as React.CSSProperties}><b>{i+1}</b><span>{displayFrequency(item.frequency)}</span><small>{filterNames[item.kind??"peak"]}</small></button>)}</div>}
          {band&&<div className="eq-inspector">
            <label>滤波器<select value={band.kind??"peak"} onChange={e=>{const kind=e.target.value as EqFilter;changeBand(bandIndex,{kind,slope_db_oct:cutFilter(kind)?band.slope_db_oct:undefined});}}>{Object.entries(filterNames).map(([kind,name])=><option key={kind} value={kind}>{name}</option>)}</select></label>
            <label>频率 (Hz)<input type="number" min={10} max={192000} step="any" required value={band.frequency} onChange={e=>changeBand(bandIndex,{frequency:Number(e.target.value)})}/></label>
            <label>增益 (dB)<input type="number" min={-30} max={30} step="any" disabled={!gainFilter(band.kind)} value={gainFilter(band.kind)?band.gain_db:0} onChange={e=>changeBand(bandIndex,{gain_db:Number(e.target.value)})}/></label>
            <label>Q 值<input type="number" min={0.1} max={30} step="any" required value={band.q} onChange={e=>changeBand(bandIndex,{q:Number(e.target.value)})}/></label>
            {pro&&cutFilter(band.kind)&&<label>斜率<select value={band.slope_db_oct??12} onChange={e=>changeBand(bandIndex,{slope_db_oct:Number(e.target.value) as DspBand["slope_db_oct"]})}>{[12,24,36,48].map(slope=><option key={slope} value={slope}>{slope} dB/oct</option>)}</select></label>}
            <label className="dsp-inline"><input type="checkbox" checked={band.enabled} onChange={e=>changeBand(bandIndex,{enabled:e.target.checked})}/>启用频段</label>
            <button type="button" aria-label="删除频段" onClick={removeBand}><Trash2 size={16}/></button>
          </div>}
          {eq.bands.some(b=>b.frequency>=previewRate/2)&&<p className="dsp-note">达到采样率一半的频段会在播放时跳过。</p>}
        </>}
        {module==="gain"&&supported&&<div className="dsp-controls"><label>前级增益 (dB)<input type="number" min={-60} max={24} step="any" value={config.preamp_db} onChange={e=>update({preamp_db:Number(e.target.value)})}/></label><label>声道平衡 <small>{Math.round(config.balance*100)}%</small><input type="range" min={-1} max={1} step={0.01} value={config.balance} onChange={e=>update({balance:Number(e.target.value)})}/></label><p className="dsp-note">前级与 EQ 的预估峰值 {+(config.preamp_db+eqPeak).toFixed(1)} dB。实际余量还受音源、响度和卷积影响。</p><button type="button" onClick={()=>update({enabled:true,preamp_db:Math.max(-60,Math.floor(-eqPeak-1))})}>为 EQ 预留约 1 dB</button></div>}
        {module==="rate"&&supported&&<div className="dsp-controls"><label>目标采样率<select value={config.sample_rate??""} onChange={e=>update({sample_rate:e.target.value?Number(e.target.value):null})}><option value="">保持源采样率</option>{rates.map(rate=><option key={rate} value={rate}>{displayFrequency(rate)}</option>)}</select></label><p className="dsp-note">由 Core 升频或降频，设备格式上限优先。</p></div>}
        {module==="stereo"&&supported&&<div className="dsp-controls"><label className="dsp-inline"><input type="checkbox" checked={!!config.stereo} onChange={e=>update({stereo:e.target.checked?defaults:null})}/>启用立体声处理</label>{config.stereo&&<><label>耳机串音 <small>{Math.round(stereo.crossfeed*100)}%</small><input type="range" min={0} max={.5} step={.01} value={stereo.crossfeed} onChange={e=>update({stereo:{...stereo,crossfeed:Number(e.target.value)}})}/></label><label>串音截止频率 (Hz)<input type="number" min={200} max={2000} value={stereo.crossover_hz} onChange={e=>update({stereo:{...stereo,crossover_hz:Number(e.target.value)}})}/></label>{([["mono","合并为单声道"],["invert_left","反转左声道极性"],["invert_right","反转右声道极性"]] as const).map(([key,title])=><label className="dsp-inline" key={key}><input type="checkbox" checked={stereo[key]} onChange={e=>update({stereo:{...stereo,[key]:e.target.checked}})}/>{title}</label>)}</>}</div>}
        {module==="convolution"&&supported&&<ImpulseLibrary api={api} onError={onError} selected={config.convolution?.response_ids} onSelect={convolution?(asset,assets)=>{
          const ids=config.convolution?.response_ids??[];
          const next=ids.filter(id=>assets.find(item=>item.id===id)?.sample_rate!==asset.sample_rate);
          if(!ids.includes(asset.id))next.push(asset.id);
          update({convolution:next.length?{response_ids:next}:null});
        }:undefined}/>}
        {module==="loudness"&&supported&&<div className="dsp-controls"><label className="dsp-inline"><input type="checkbox" checked={config.normalize} onChange={e=>update({normalize:e.target.checked})}/>自动响度均衡</label><label>目标响度 (LUFS)<input type="number" min={-40} max={-5} step="any" value={config.target_lufs} onChange={e=>update({target_lufs:Number(e.target.value)})}/></label><p className="dsp-note">{zone.track?.integrated_lufs!=null?`当前歌曲实测 ${zone.track.integrated_lufs.toFixed(1)} LUFS`:"当前歌曲暂无响度分析"}</p>{zone.track?.source==="local"&&<button type="button" disabled={analyzing} onClick={()=>{setAnalyzing(true);void api.post(`/library/tracks/${encodeURIComponent(zone.track!.id)}/analyze`).catch(error=>{setAnalyzing(false);onError(error);});}}>{analyzing?"响度分析已排队":"分析当前歌曲"}</button>}</div>}
        {module==="fades"&&<div className="dsp-controls">{transitionsSupported?<><label>曲首淡入 (ms)<input type="number" min={0} max={10000} value={fades.fade_in_ms} onChange={e=>setFades({...fades,fade_in_ms:Number(e.target.value)})}/></label><label>曲尾淡出 (ms)<input type="number" min={0} max={10000} value={fades.fade_out_ms} onChange={e=>setFades({...fades,fade_out_ms:Number(e.target.value)})}/></label><button type="button" onClick={()=>setFades({fade_in_ms:80,fade_out_ms:120})}>均衡预设</button></>:<p className="dsp-note">此输出不支持淡入淡出。</p>}</div>}
      </section>
    </div>
  </form>;
}

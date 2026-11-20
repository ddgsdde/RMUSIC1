import type {MusicIdentity,VersionQualityEvidence} from '@r-music/sdk';
export function VersionEvidence({evidence,identity}:{evidence?:VersionQualityEvidence;identity?:MusicIdentity}){
 if(!evidence)return null;
 const file=evidence.file_record;
 return <details className="version-evidence"><summary>关系与质量依据</summary><dl><div><dt>存储</dt><dd>{{platform:'平台音源',local_file:'本地文件',core_cache:'Core缓存副本'}[evidence.storage]}</dd></div><div><dt>来源</dt><dd>{evidence.source}</dd></div>{evidence.provider_declared?.label&&<div><dt>平台声明</dt><dd>{evidence.provider_declared.label} · 尚未据此认证音质</dd></div>}{file.format&&<div><dt>{file.verified?'文件记录':'来源记录'}</dt><dd>{[file.format.toUpperCase(),file.sample_rate?`${file.sample_rate} Hz`:null,file.bit_depth?`${file.bit_depth} bit`:null,file.channels?`${file.channels} 声道`:null,file.duration>0?`${file.duration.toFixed(1)} 秒`:null].filter(Boolean).join(' · ')}</dd></div>}</dl>
 {['work','recording','release'].map(kind=><div key={kind}>{identity?.entities.filter(v=>v.kind===kind).map((v,i)=><p key={v.id+v.basis+i}>{{work:'作品',recording:'录音',release:'发行'}[kind]} · {v.title}<small> · {v.basis==='manual'?'人工确认':v.namespace}</small></p>)}</div>)}<p className="fine-print">实际输出以当前音区链路报告为准；版本标签不代表音质认证。</p></details>;
}

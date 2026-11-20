import {useEffect,useState} from "react";
import {Download,Pause,Play,RefreshCw} from "lucide-react";
import {presentationVisible} from "./presentationVisibility";
import {invokeNative} from "./native";
import {formatBytes} from "./browserCache";
interface Task {id:string;track:{title?:string;id:string};status:string;received:number;total:number;error?:string}
const labels:Record<string,string>={queued:"等待下载",downloading:"正在下载",verifying:"正在校验",complete:"已完成",interrupted:"等待连接原账号后继续",paused:"已暂停",failed:"下载未完成"};
export function NativeDownloads({api,onError}:{api:{baseUrl:string;token:string};onError:(error:unknown)=>void}){
 const [tasks,setTasks]=useState<Task[]>([]);
 useEffect(()=>{let active=true,busy=false;const read=async()=>{if(busy||!presentationVisible())return;busy=true;try{const value=await invokeNative<{items:Task[]}>("native_download_tasks",{address:api.baseUrl});if(active)setTasks(value.items);}catch(error){if(active)onError(error);}finally{busy=false;}};void read();const timer=setInterval(read,1500);return()=>{active=false;clearInterval(timer);};},[onError,api.baseUrl,api.token]);
 async function action(task:Task,action:string){try{await invokeNative("native_download_action",{id:task.id,action});setTasks(old=>old.map(item=>item.id===task.id?{...item,status:action==="pause"?"paused":"queued",error:undefined}:item));}catch(error){onError(error);}}
 if(!tasks.length)return null;
 return <section className="native-download-tasks" aria-label="本机下载任务"><h3>下载任务</h3><p className="fine-print">退出后保留已确认片段；重新打开并连接原账号后继续。手动暂停的任务保持暂停。</p>{tasks.map(task=>{const running=["queued","downloading","verifying"].includes(task.status);return <article key={task.id}><Download size={18}/><div><strong>{task.track.title||task.track.id}</strong><span>{labels[task.status]||task.status} · {formatBytes(task.received)}{task.total>0?` / ${formatBytes(task.total)}`:""}</span>{task.error&&<small>{task.error}</small>}{running&&<progress aria-label={`${task.track.title||"歌曲"}下载进度`} max={Math.max(1,task.total)} value={task.received}/>}</div>{task.status!=="complete"&&<button aria-label={running?"暂停下载":"继续下载"} disabled={task.status==="verifying"} onClick={()=>void action(task,running?"pause":"resume")}>{running?<Pause size={18}/>:task.status==="failed"?<RefreshCw size={18}/>:<Play size={18}/>}</button>}</article>;})}</section>;
}

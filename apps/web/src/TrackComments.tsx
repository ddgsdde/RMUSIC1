import {useEffect,useRef,useState} from "react";
import {ChevronLeft,ChevronRight,LoaderCircle,RefreshCw,ThumbsUp} from "lucide-react";
import type {RMusicClient,TrackComments as Comments} from "@r-music/sdk";
import {cachedResource,loadResource,useResourceRefresh} from "./resourceCache";
import {IconButton} from "./components";

export function TrackComments({api,id}:{api:RMusicClient;id:string}){
 const [kind,setKind]=useState<"recommend"|"hot"|"new">("recommend"),[page,setPage]=useState(1),[cursors,setCursors]=useState<string[]>([""]),[revision,setRevision]=useState(0);
 const [snapshot,setSnapshot]=useState<{api:RMusicClient;key:string;value:Comments}|null>(null),[error,setError]=useState(""),[loading,setLoading]=useState(false);
 const pulse=useResourceRefresh(),stamp=useRef(0),cursor=cursors[page-1]||"",key=`comments:${id}:${kind}:${page}:${cursor}`;
 const data=snapshot?.api===api&&snapshot.key===key?snapshot.value:cachedResource<Comments>(api,key);
 useEffect(()=>{let active=true;if(document.hidden)return;const force=stamp.current!==revision;stamp.current=revision;setLoading(true);setError("");
  const query=new URLSearchParams({kind,page:String(page),page_size:"20",cursor,refresh:String(force)});
  void loadResource(api,key,()=>api.get<Comments>(`/library/tracks/${encodeURIComponent(id)}/comments?${query}`),kind==="new"?30000:300000,force).then(value=>{if(active)setSnapshot({api,key,value});}).catch(cause=>{if(active)setError(cause.message);}).finally(()=>{if(active)setLoading(false);});return()=>{active=false;};
 },[api,id,kind,page,cursor,key,revision,pulse]);
 return <section className="track-comments" aria-label="乐评与评论">
  <div className="comment-toolbar"><div className="comment-tabs" role="group" aria-label="评论排序">{([["recommend","推荐乐评"],["hot","热门"],["new","最新"]] as const).map(([value,label])=><button key={value} aria-pressed={kind===value} className={kind===value?"selected":""} onClick={()=>{setKind(value);setPage(1);setCursors([""]);}}>{label}</button>)}</div><IconButton label="获取最新评论" disabled={loading} onClick={()=>setRevision(v=>v+1)}><RefreshCw size={17} className={loading?"spin":""}/></IconButton></div>
  <div className="comment-status muted" aria-live="polite">{loading?<><LoaderCircle size={14} className="spin"/>正在更新…</>:data?.fetched_at?`更新于 ${new Date(data.fetched_at*1000).toLocaleTimeString()}${data.total!==undefined?` · ${data.total.toLocaleString()} 条`:""}`:"仅读取音源公开内容"}</div>
  {error&&<p role="alert" className="notice">更新失败{data?"，已保留上次内容":""}：{error}<button className="text-button" onClick={()=>setRevision(v=>v+1)}>重试</button></p>}
  {data?.notice&&<p className="notice">{data.notice}</p>}
  {data?.supported&&!data.items.length&&!loading&&<p className="muted">此分类暂时没有评论。</p>}
  <div className="comment-list">{data?.items.map((item,index)=><article key={`${item.id}:${index}`}><header><strong>{item.author||"音乐听友"}</strong>{item.published_at>0&&<time dateTime={new Date(item.published_at*1000).toISOString()}>{new Date(item.published_at*1000).toLocaleDateString()}</time>}</header><p>{item.text}</p><footer><span><ThumbsUp size={13}/>{item.likes.toLocaleString()}</span></footer>{item.reply_count>0&&<details className="comment-replies"><summary>展开回复 · {item.reply_count.toLocaleString()}</summary>{item.replies?.map((reply,index)=><article key={`${reply.id}:${index}`}><header><strong>{reply.author||"音乐听友"}</strong>{reply.published_at>0&&<time>{new Date(reply.published_at*1000).toLocaleDateString()}</time>}</header>{reply.reply_to&&<small className="muted">回复 {reply.reply_to}</small>}<p>{reply.text}</p></article>)}{(item.replies?.length||0)<item.reply_count&&<p className="fine-print">音源当前返回 {item.replies?.length||0} 条回复，未完整返回其余内容。<button className="text-button" disabled={loading} onClick={()=>setRevision(v=>v+1)}>重新获取</button></p>}</details>}</article>)}</div>
  {data?.supported&&<div className="comment-pagination"><IconButton label="评论上一页" disabled={loading||page===1} onClick={()=>setPage(v=>v-1)}><ChevronLeft size={18}/></IconButton><span>第 {page} 页</span><IconButton label="评论下一页" disabled={loading||!data.has_more||page>=1000} onClick={()=>{setCursors(old=>[...old.slice(0,page),data.next_cursor||""]);setPage(v=>v+1);}}><ChevronRight size={18}/></IconButton></div>}
 </section>;
}

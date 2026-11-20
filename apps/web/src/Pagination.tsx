import {useEffect,useState} from "react";
function read(key:string){try{const value=JSON.parse(sessionStorage.getItem(key)||"null");if(Number.isSafeInteger(value?.page)&&value.page>0&&[25,50,100].includes(value.size))return {key,page:value.page,size:value.size};}catch{}return {key,page:1,size:50};}
export function usePaging(key:string){
 const [saved,setSaved]=useState(()=>read(key));
 const value=saved.key===key?saved:{key,page:1,size:saved.size};
 useEffect(()=>{if(saved.key!==key)setSaved(value);},[key]);
 function change(page:number,size=value.size){const next={key,page:Math.max(1,page),size};sessionStorage.setItem(key,JSON.stringify(next));setSaved(next);}
 return {...value,change};
}
export function Pagination({page,size,total,busy,onChange}:{page:number;size:number;total:number;busy:boolean;onChange:(page:number,size?:number)=>void}){
 const pages=Math.max(1,Math.ceil(total/size)),[jump,setJump]=useState(String(page));
 useEffect(()=>setJump(String(page)),[page]);
 function go(){const target=Number(jump);if(Number.isSafeInteger(target))onChange(Math.max(1,Math.min(pages,target)));else setJump(String(page));}
 return <nav className="pagination-controls" aria-label="列表分页">
  <span>共 {total.toLocaleString()} 项 · 第 {page} / {pages} 页</span>
  <button type="button" disabled={busy||page<=1} onClick={()=>onChange(page-1)}>上一页</button>
  <button type="button" disabled={busy||page>=pages} onClick={()=>onChange(page+1)}>下一页</button>
  <label>每页<select aria-label="每页数量" value={size} disabled={busy} onChange={event=>onChange(1,Number(event.target.value))}>{[25,50,100].map(count=><option key={count} value={count}>{count}</option>)}</select></label>
  <label>跳至<input aria-label="跳转页码" type="number" min={1} max={pages} value={jump} disabled={busy} onChange={event=>setJump(event.target.value)} onKeyDown={event=>{if(event.key==='Enter'){event.preventDefault();go();}}}/></label><button type="button" disabled={busy} onClick={go}>跳转</button>
 </nav>;
}

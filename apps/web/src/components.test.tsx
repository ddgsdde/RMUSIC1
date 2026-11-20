// @vitest-environment jsdom
import {act,useState} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {RMusicClient} from "@r-music/sdk";
import {Cover,Modal} from "./components";

let host:HTMLDivElement,root:Root;
beforeEach(()=>{
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
 host=document.createElement("div");document.body.append(host);root=createRoot(host);
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();vi.restoreAllMocks();});

it("nested dialogs keep keyboard focus inside, close only the top dialog and restore the opener",async()=>{
 function Example(){const [open,setOpen]=useState(false),[nested,setNested]=useState(false);return <><button onClick={()=>setOpen(true)}>打开设置</button>{open&&<Modal title="设置" onClose={()=>setOpen(false)}><input aria-label="名称"/><button onClick={()=>setNested(true)}>更多</button>{nested&&<Modal title="更多" onClose={()=>setNested(false)}><button>完成</button></Modal>}</Modal>}<div hidden><Modal active={false} title="隐藏的离线播放器" onClose={()=>{}}><audio/></Modal></div></>;}
 await act(async()=>root.render(<Example/>));
 const opener=host.querySelector('button')!;opener.focus();await act(async()=>opener.click());
 const outer=host.querySelector<HTMLElement>('[role=dialog]')!;
 expect(document.activeElement).toBe(outer);
 const buttons=outer.querySelectorAll('button'),more=buttons[1];
 more.focus();document.dispatchEvent(new KeyboardEvent('keydown',{key:'Tab',bubbles:true,cancelable:true}));
 expect(document.activeElement).toBe(buttons[0]);
 document.dispatchEvent(new KeyboardEvent('keydown',{key:'Tab',shiftKey:true,bubbles:true,cancelable:true}));
 expect(document.activeElement).toBe(more);
 await act(async()=>more.click());
 expect(host.querySelectorAll('[aria-modal=true]')).toHaveLength(2);
 await act(async()=>document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true})));
 expect(host.querySelectorAll('[aria-modal=true]')).toHaveLength(1);expect(document.activeElement).toBe(more);
 await act(async()=>document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true})));
 expect(host.querySelector('[aria-modal=true]')).toBeNull();expect(document.activeElement).toBe(opener);expect(document.body.style.overflow).toBe('');
});

it("a late cover response after leaving a page does not allocate an unreleased object URL",async()=>{
 let finish!:(response:Response)=>void;
 vi.stubGlobal('fetch',vi.fn(()=>new Promise<Response>(resolve=>{finish=resolve;})));
 const create=vi.fn(()=>"blob:cover");vi.stubGlobal('URL',class extends URL{static createObjectURL=create;static revokeObjectURL=vi.fn();});
 await act(async()=>root.render(<Cover api={new RMusicClient('http://core')} track={{cover_url:'http://core/cover',title:'歌曲',album:'专辑'}}/>));
 await act(async()=>root.render(null));
 await act(async()=>finish({ok:true,blob:async()=>new Blob(['cover'])} as Response));
 expect(create).not.toHaveBeenCalled();
});

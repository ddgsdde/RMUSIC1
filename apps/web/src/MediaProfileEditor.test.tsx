// @vitest-environment jsdom
import {act} from 'react';
import {createRoot, type Root} from 'react-dom/client';
import {beforeEach, afterEach, expect, it, vi} from 'vitest';
import {RMusicClient} from '@r-music/sdk';
import {MediaProfileEditor} from './MediaProfileEditor';
let host: HTMLDivElement, root: Root;
beforeEach(() => {vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true); host = document.createElement('div'); document.body.append(host); root = createRoot(host);});
afterEach(async () => {await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals();});
const snapshot = {revision: 3, profile: {description: 'Human', genre: 'Pop'}, manual: {description: 'Human'}};
it('只提交锁定字段与明确解锁字段，并保留修订号', async () => {
  const api = new RMusicClient('http://core', 'token'), saved = vi.fn(), close = vi.fn();
  const request = vi.spyOn(api, 'request').mockResolvedValue(snapshot);
  await act(async () => root.render(<MediaProfileEditor api={api} kind="artist" id="id" saved={saved} close={close}/>));
  await act(async () => host.querySelector<HTMLInputElement>('[aria-label="锁定简介"]')!.click());
  await act(async () => host.querySelector('form')!.dispatchEvent(new Event('submit', {bubbles:true,cancelable:true})));
  expect(JSON.parse(request.mock.calls[1][1]!.body as string)).toEqual({expected_revision:3, fields:{}, unlock:['description']});
  expect(saved).toHaveBeenCalledOnce(); expect(close).toHaveBeenCalledOnce();
});
it('冲突保留编辑内容，重新载入后使用新修订号', async () => {
  const api = new RMusicClient('http://core','token'), close = vi.fn();
  const request = vi.spyOn(api,'request').mockResolvedValueOnce(snapshot).mockRejectedValueOnce(new Error('资料已在其他设备修改')).mockResolvedValueOnce({...snapshot, revision:4});
  await act(async () => root.render(<MediaProfileEditor api={api} kind="artist" id="id" saved={vi.fn()} close={close}/>));
  await act(async () => host.querySelector('form')!.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true})));
  expect(host.querySelector('[role=alert]')!.textContent).toContain('其他设备');
  expect(host.querySelector<HTMLTextAreaElement>('textarea')!.value).toBe('Human'); expect(close).not.toHaveBeenCalled();
  await act(async () => [...host.querySelectorAll('button')].find(x=>x.textContent==='重新载入')!.click());
  request.mockResolvedValue({});
  await act(async () => host.querySelector('form')!.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true})));
  expect(JSON.parse(request.mock.calls[3][1]!.body as string).expected_revision).toBe(4);
});

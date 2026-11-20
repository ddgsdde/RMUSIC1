import {useEffect, useState} from 'react';
import type {RMusicClient} from '@r-music/sdk';
import {Modal} from './components';

const fields: Record<string, string> = {
  image_url: '封面／头像地址', description: '简介', genre: '风格', language: '语言',
  release_date: '发行日期', label: '唱片公司', country: '地区', birthday: '生日',
  birthplace: '出生地', foreign_name: '外文名', alias: '别名',
};
type Snapshot = {revision: number; manual: Record<string, string>; profile: Record<string, string>};
type Props = {api: RMusicClient; kind: string; id: string; close: () => void; saved: () => void};

export function MediaProfileEditor({api, kind, id, close, saved}: Props) {
  const [data, setData] = useState<Snapshot>();
  const [values, setValues] = useState<Record<string, string>>({});
  const [locked, setLocked] = useState(new Set<string>());
  const [busy, setBusy] = useState(false), [loading, setLoading] = useState(true);
  const [error, setError] = useState(''), [reload, setReload] = useState(0);
  const path = `/library/catalog/${kind}/${encodeURIComponent(id)}/profile`;
  useEffect(() => {
    const abort = new AbortController();
    setLoading(true); setError('');
    api.request<Snapshot>(path, {signal: abort.signal}).then(value => {
      if (abort.signal.aborted) return;
      setData(value); setValues(value.profile); setLocked(new Set(Object.keys(value.manual)));
    }).catch(e => { if (!abort.signal.aborted) setError(e.message); })
      .finally(() => { if (!abort.signal.aborted) setLoading(false); });
    return () => abort.abort();
  }, [api, path, reload]);
  function change(key: string, value: string) {
    setValues(old => ({...old, [key]: value}));
    setLocked(old => new Set([...old, key]));
  }
  async function save() {
    if (!data || busy || loading) return;
    setBusy(true); setError('');
    try {
      await api.request(path, {method: 'PATCH', body: JSON.stringify({
        expected_revision: data.revision,
        fields: Object.fromEntries([...locked].map(key => [key, values[key] || ''])),
        unlock: Object.keys(data.manual).filter(key => !locked.has(key)),
      })});
      saved(); close();
    } catch (e) { setError(e instanceof Error ? e.message : '资料保存失败'); }
    finally { setBusy(false); }
  }
  return <Modal title="编辑资料" onClose={() => { if (!busy) close(); }}>
    <p className="muted">修改的字段会保留在 Core。取消锁定后，恢复采用平台资料。</p>
    <form onSubmit={e => {e.preventDefault(); void save();}}>
      <fieldset disabled={loading || busy} style={{border: 0, padding: 0, margin: 0}}>
        {Object.entries(fields).map(([key, label]) => <div key={key} className="metadata-edit-field">
          <span><label htmlFor={`profile-${key}`}>{label}</label><label>
            <input type="checkbox" aria-label={`锁定${label}`} checked={locked.has(key)} onChange={e => setLocked(old => {
              const next = new Set(old);
              if (e.target.checked) next.add(key); else next.delete(key);
              return next;
            })}/>锁定
          </label></span>
          {key === 'description'
            ? <textarea id={`profile-${key}`} rows={7} value={values[key] || ''} onChange={e => change(key, e.target.value)}/>
            : <input id={`profile-${key}`} value={values[key] || ''} onChange={e => change(key, e.target.value)}/>}
        </div>)}
      </fieldset>
      {error && <p role="alert">{error}</p>}
      <button type="button" disabled={busy || loading} onClick={() => setReload(n => n + 1)}>重新载入</button>{' '}
      <button disabled={!data || loading || busy}>{busy ? '保存中…' : '保存资料'}</button>
    </form>
  </Modal>;
}

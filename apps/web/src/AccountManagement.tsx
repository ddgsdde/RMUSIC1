import { useEffect, useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import type { RMusicClient, User } from "@r-music/sdk";
import { IconButton, Modal } from "./components";
interface Session {
  id: string;
  created_at?: number;
  expires_at?: number;
  current?: boolean;
  device_name?: string | null;
  device_type?: string | null;
}
const deviceTypes:Record<string,string>={macos:"Mac",android:"Android",windows:"Windows",linux:"Linux",browser:"浏览器"};
export function AccountManagement({
  api,
  user,
  notify,
  onError,
}: {
  api: RMusicClient;
  user: User;
  notify: (message: string) => void;
  onError: (error: unknown) => void;
}) {
  const [users, setUsers] = useState<User[]>([]);
  const [sessions, setSessions] = useState<Session[]>([]);
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const reload = () => {
    if (user.role === "admin")
      api
        .get<User[] | { items: User[] }>("/auth/users")
        .then((r) => setUsers(Array.isArray(r) ? r : r.items))
        .catch(onError);
    api
      .get<Session[] | { items: Session[] }>("/auth/sessions")
      .then((r) => setSessions(Array.isArray(r) ? r : r.items))
      .catch(onError);
  };
  useEffect(() => {
    reload();
  }, [api, user.role]);
  return (
    <>
      {user.role === "admin" && (
        <section className="settings-section">
          <div className="section-heading">
            <h2>家庭账号</h2>
            <button onClick={() => setAdding(true)}>
              <Plus size={16} />
              创建账号
            </button>
          </div>
          <div className="settings-list">
            {users.map((item) => (
              <div key={item.id}>
                <strong>{item.username}</strong>
                <small>
                  {item.role === "admin" ? "管理员" : "家庭成员"}
                  {item.totp_enabled ? " · 两步验证" : ""}
                </small>
              </div>
            ))}
          </div>
        </section>
      )}
      <section className="settings-section">
        <h2>登录会话</h2>
        <div className="settings-list">
          {sessions.map((session) => (
            <div key={session.id}>
              <div>
                <strong>{session.device_name || "未标识设备"}</strong>
                <small>{deviceTypes[session.device_type ?? ""] || "设备类型未知"}{session.current ? " · 当前会话" : ""}</small>
                <small>
                  {session.created_at
                    ? new Date(session.created_at * 1000).toLocaleString()
                    : "会话已登录"}
                  {session.expires_at
                    ? ` · 有效至 ${new Date(session.expires_at * 1000).toLocaleString()}`
                    : ""}
                </small>
              </div>
              <IconButton
                label="撤销登录会话"
                onClick={() =>
                  api
                    .delete(`/auth/sessions/${session.id}`)
                    .then(() => {
                      notify("会话已撤销");
                      reload();
                    })
                    .catch(onError)
                }
              >
                <Trash2 size={17} />
              </IconButton>
            </div>
          ))}
        </div>
      </section>
      {adding && (
        <Modal title="创建家庭账号" onClose={() => setAdding(false)}>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              api
                .post("/auth/users", { username: name, password })
                .then(() => {
                  setAdding(false);
                  setName("");
                  setPassword("");
                  notify("家庭账号已创建");
                  reload();
                })
                .catch(onError);
            }}
          >
            <label>
              用户名
              <input
                required
                maxLength={80}
                autoComplete="off"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </label>
            <label>
              初始密码
              <input
                required
                minLength={12}
                type="password"
                autoComplete="new-password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
            </label>
            <button className="primary">创建账号</button>
          </form>
        </Modal>
      )}
    </>
  );
}

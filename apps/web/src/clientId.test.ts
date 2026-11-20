import { afterEach,expect,it,vi } from "vitest";
import { clientId } from "./clientId";
afterEach(()=>vi.unstubAllGlobals());
it("generates distinct version 4 IDs when HTTP LAN context has no randomUUID",()=>{
  const getRandomValues=crypto.getRandomValues.bind(crypto);
  vi.stubGlobal("crypto",{getRandomValues});
  const first=clientId(),second=clientId();
  expect(first).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  expect(second).not.toBe(first);
});

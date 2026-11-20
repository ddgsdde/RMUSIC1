import { clientId } from "./clientId";
import { useEffect, useState } from "react";
// One tab owns this device output. Other tabs remain full controllers.
export function useOutputOwner(endpointId: string) {
  const [owner, setOwner] = useState(false);
  useEffect(() => {
    let active = true;
    let held = false;
    let release: (() => void) | undefined;
    const key = `rmusic-output-${endpointId}`;
    if (navigator.locks) {
      const attempt = () => {
        if (held || !active) return;
        navigator.locks.request(key, { ifAvailable: true }, async (lock) => {
          if (!active || !lock) return;
          held = true;
          setOwner(true);
          await new Promise<void>((resolve) => {
            release = resolve;
          });
          held = false;
          if (active) setOwner(false);
        });
      };
      attempt();
      const timer = setInterval(attempt, 2000);
      return () => {
        active = false;
        clearInterval(timer);
        release?.();
      };
    }
    if (typeof BroadcastChannel === "undefined") {
      setOwner(true);
      return () => {
        active = false;
      };
    }
    const channel = new BroadcastChannel(key);
    const id = clientId();
    const members = new Map<string, number>();
    const elect = () => {
      const now = Date.now();
      members.set(id, now);
      for (const [member, time] of members)
        if (now - time > 3500) members.delete(member);
      setOwner([...members.keys()].sort()[0] === id);
      channel.postMessage({ id, time: now });
    };
    channel.onmessage = (event) => {
      if (event.data?.id && typeof event.data.time === "number")
        members.set(event.data.id, event.data.time);
    };
    channel.postMessage({ id, time: Date.now() });
    const initial = setTimeout(elect, 350);
    const timer = setInterval(elect, 1000);
    return () => {
      active = false;
      clearTimeout(initial);
      clearInterval(timer);
      channel.close();
    };
  }, [endpointId]);
  return owner;
}

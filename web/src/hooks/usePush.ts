import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import type { PushDelivery, PushDevice } from "@/lib/api";
import { currentSubscription, environment, pushSupport, turnOff, turnOn } from "@/lib/push";
import type { PushEnvironment, PushSupport } from "@/lib/push";

/** Every query about notifications, to refetch them at once. */
export const PUSH = ["push"];
const DEVICES = [...PUSH, "devices"];
const KEY = [...PUSH, "key"];
const HERE = [...PUSH, "here"];

export interface Push {
  env: PushEnvironment;
  support: PushSupport;
  /** The server can send: it has a database and its secrets key. */
  serverReady: boolean;
  /** Why it cannot, when it says. */
  unavailable?: string;
  /** This device is subscribed, and Niles knows it. */
  on: boolean;
  /** This device's endpoint, to mark it in the list. */
  here: string | null;
  devices?: PushDevice[];
  /** The key is here, so a tap can go straight to the question. */
  canAsk: boolean;
  busy: boolean;
  error?: string;
  tested?: PushDelivery;
  enable: () => void;
  disable: () => void;
  remove: (endpoint: string) => void;
  test: () => void;
}

function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/**
 * This device and the person's others, for the profile's card and the
 * nudge under a job. Shared through the query keys, so turning it on
 * in one takes the other's offer away.
 */
export function usePush(): Push {
  const queryClient = useQueryClient();
  // Read again after each change: the answer to the browser's question
  // is part of it.
  const [env, setEnv] = useState(environment);
  const support = pushSupport(env);
  const devices = useQuery({ queryKey: DEVICES, queryFn: api.pushDevices, retry: false });
  const key = useQuery({
    queryKey: KEY,
    queryFn: api.pushKey,
    retry: false,
    staleTime: Infinity,
    enabled: support === "available" && devices.isSuccess,
  });
  const here = useQuery({
    queryKey: HERE,
    queryFn: async () => (await currentSubscription())?.endpoint ?? null,
    enabled: env.hasPush,
  });
  const [enabling, setEnabling] = useState(false);
  const [enableError, setEnableError] = useState<string>();

  const refresh = () => {
    setEnv(environment());
    void queryClient.invalidateQueries({ queryKey: PUSH });
  };
  const disable = useMutation({ mutationFn: turnOff, onSettled: refresh });
  const remove = useMutation({
    mutationFn: async (endpoint: string) => {
      // This device's own entry is this device turned off, or the
      // browser would keep a subscription nobody sends to.
      if (endpoint === here.data) return turnOff();
      return api.unsubscribePush(endpoint);
    },
    onSettled: refresh,
  });
  const test = useMutation({ mutationFn: api.testPush, onSettled: refresh });

  const enable = () => {
    if (!key.data || enabling) return;
    setEnabling(true);
    setEnableError(undefined);
    // Not a mutation: `turnOn` has to ask the browser in this same tap,
    // before anything awaits.
    turnOn(key.data.key)
      .catch((e) => setEnableError(messageOf(e)))
      .finally(() => {
        setEnabling(false);
        refresh();
      });
  };

  const list = devices.data?.devices;
  const on =
    env.permission === "granted" &&
    Boolean(here.data && list?.some((d) => d.endpoint === here.data));
  return {
    env,
    support,
    serverReady: devices.isSuccess,
    unavailable: devices.error?.message,
    on,
    here: here.data ?? null,
    devices: list,
    canAsk: Boolean(key.data),
    busy: enabling || disable.isPending || remove.isPending || test.isPending,
    error:
      enableError ??
      disable.error?.message ??
      remove.error?.message ??
      test.error?.message ??
      key.error?.message,
    tested: test.data,
    enable,
    disable: () => disable.mutate(),
    remove: (endpoint) => remove.mutate(endpoint),
    test: () => test.mutate(),
  };
}

import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChatView } from "@/components/ChatView";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import type { Exchange } from "@/lib/api";

const KEY = ["chat"];

/**
 * The chat, kept on the server so it survives a reload and a trip to
 * another tab, and so the next message has the last one as context.
 */
export function ChatPage() {
  const queryClient = useQueryClient();
  const history = useQuery({
    queryKey: KEY,
    queryFn: api.chat,
    // Only this page writes to it, and a refetch landing mid-send would
    // briefly drop the message being answered.
    refetchOnWindowFocus: false,
    staleTime: Infinity,
  });
  const [pending, setPending] = useState<string>();

  const send = useMutation({
    mutationFn: api.sendChat,
    onMutate: (text) => setPending(text),
    onSuccess: ({ reply }, said) => {
      queryClient.setQueryData<Exchange[]>(KEY, (old) => [...(old ?? []), { said, reply }]);
    },
    onSettled: () => setPending(undefined),
  });
  const forget = useMutation({
    mutationFn: api.forgetChat,
    onSuccess: () => queryClient.setQueryData<Exchange[]>(KEY, []),
  });

  if (history.isLoading) {
    return <Skeleton className="h-64 w-full" />;
  }
  if (history.isError) {
    return (
      <p className="text-muted-foreground text-sm">
        Can't reach Niles: {history.error.message}
      </p>
    );
  }

  return (
    <ChatView
      exchanges={history.data ?? []}
      pending={pending}
      error={send.error?.message ?? forget.error?.message}
      onSend={(text) => send.mutate(text)}
      onForget={() => forget.mutate()}
      onDictate={async (audio) => (await api.dictate(audio)).text}
    />
  );
}

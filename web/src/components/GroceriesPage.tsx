import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { GroceryList } from "@/components/GroceryList";
import { NemligOrder } from "@/components/NemligOrder";
import { NemligPicker } from "@/components/NemligPicker";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import type {
  GroceryEdit,
  GroceryItem,
  GroceryList as List,
  NemligProduct,
  NemligSent,
} from "@/lib/api";

const KEY = ["groceries"];

/**
 * The shopping list, kept current with what is said in the kitchen.
 */
export function GroceriesPage() {
  const queryClient = useQueryClient();
  const list = useQuery({
    queryKey: KEY,
    queryFn: api.groceries,
    // Somebody else adds by voice while you stand in the shop, and a
    // list that only changes when you touch it is the wrong list.
    refetchInterval: 5_000,
  });
  const refresh = () => queryClient.invalidateQueries({ queryKey: KEY });

  const add = useMutation({ mutationFn: api.addGrocery, onSuccess: refresh });
  const edit = useMutation({
    mutationFn: ({ id, change }: { id: number; change: GroceryEdit }) =>
      api.editGrocery(id, change),
    // Ticked at once, not a round trip later: on a phone in a shop the
    // round trip is the slow part.
    onMutate: async ({ id, change }) => {
      if (change.checked === undefined) return;
      await queryClient.cancelQueries({ queryKey: KEY });
      queryClient.setQueryData<List>(KEY, (old) =>
        old && {
          ...old,
          items: old.items.map((i) =>
            i.id === id
              ? { ...i, checked_at: change.checked ? new Date().toISOString() : undefined }
              : i,
          ),
        },
      );
    },
    onSettled: refresh,
  });
  const remove = useMutation({ mutationFn: api.removeGrocery, onSuccess: refresh });
  const clear = useMutation({ mutationFn: api.clearGroceries, onSuccess: refresh });
  const [picking, setPicking] = useState<GroceryItem>();
  const choose = useMutation({
    mutationFn: ({ id, product }: { id: number; product: NemligProduct | null }) =>
      api.chooseNemlig(id, product),
    onSuccess: refresh,
  });

  // The order sheet: open once the list has gone to the basket.
  const [sent, setSent] = useState<NemligSent>();
  const send = useMutation({ mutationFn: api.nemligSend, onSuccess: setSent });
  const delivery = useQuery({
    queryKey: ["nemlig-delivery"],
    queryFn: api.nemligDelivery,
    enabled: sent !== undefined,
    retry: false,
    staleTime: 60_000,
  });
  const reserve = useMutation({
    mutationFn: api.nemligReserve,
    onSuccess: (basket) => {
      setSent((s) => s && { ...s, basket });
      queryClient.invalidateQueries({ queryKey: ["nemlig-delivery"] });
    },
  });

  if (list.isLoading) {
    return <Skeleton className="h-64 w-full" />;
  }
  if (list.isError) {
    return (
      <p className="text-muted-foreground text-sm">
        Can't reach the list: {list.error.message}
      </p>
    );
  }

  const toggle = (item: GroceryItem) =>
    edit.mutate({ id: item.id, change: { checked: !item.checked_at } });

  return (
    <>
    <GroceryList
      items={list.data?.items ?? []}
      usual={list.data?.usual ?? []}
      error={
        add.error?.message ??
        edit.error?.message ??
        remove.error?.message ??
        clear.error?.message ??
        choose.error?.message ??
        send.error?.message
      }
      onAdd={(name) => add.mutate(name)}
      onToggle={toggle}
      onEdit={(item, change) => edit.mutate({ id: item.id, change })}
      onRemove={(item) => remove.mutate(item.id)}
      onClear={() => clear.mutate()}
      onPick={list.data?.nemlig ? setPicking : undefined}
      onSend={list.data?.nemlig ? () => send.mutate() : undefined}
      sending={send.isPending}
    />
    <NemligOrder
      sent={sent}
      days={delivery.data}
      daysError={delivery.error?.message}
      reserving={reserve.isPending ? reserve.variables : undefined}
      reserveError={reserve.error?.message}
      onReserve={(slotId) => reserve.mutate(slotId)}
      onClose={() => {
        setSent(undefined);
        reserve.reset();
      }}
    />
    <NemligPicker
      item={picking}
      search={api.nemligSearch}
      onChoose={(item, product) => {
        choose.mutate({ id: item.id, product });
        setPicking(undefined);
      }}
      onClose={() => setPicking(undefined)}
    />
    </>
  );
}

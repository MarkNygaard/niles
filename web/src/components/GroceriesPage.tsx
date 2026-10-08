import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { GroceryList } from "@/components/GroceryList";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import type { GroceryEdit, GroceryItem, GroceryList as List } from "@/lib/api";

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
    <GroceryList
      items={list.data?.items ?? []}
      usual={list.data?.usual ?? []}
      error={
        add.error?.message ??
        edit.error?.message ??
        remove.error?.message ??
        clear.error?.message
      }
      onAdd={(name) => add.mutate(name)}
      onToggle={toggle}
      onEdit={(item, change) => edit.mutate({ id: item.id, change })}
      onRemove={(item) => remove.mutate(item.id)}
      onClear={() => clear.mutate()}
    />
  );
}

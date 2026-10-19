import { useEffect, useState } from "react";
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Bell,
  CalendarClock,
  FileText,
  Footprints,
  ShieldCheck,
  UsersRound,
} from "lucide-react";
import { GroceryDeliveryCard, arrivesToday } from "@/components/GroceryDeliveryCard";
import { PairPhoneCard, shouldOffer } from "@/components/PairPhoneCard";
import { TRAINING_TODAY, TodaysRunCard } from "@/components/TodaysRunCard";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import type { AppNotification, NotificationList } from "@/lib/api";
import { NOTIFICATIONS, PAGES, hrefOf, newestId } from "@/lib/notifications";
import { relativeTime } from "@/lib/skills";
import { cn } from "@/lib/utils";

/**
 * What is the signed-in person's own: what matters today at the top,
 * and below it everything Niles told them, newest first.
 *
 * Opening it reads what is there, and so does a notice that arrives
 * while it is open. What was unread when it was first shown keeps its
 * dot for the visit, so the page still says what is new.
 */
export function NotificationsPage() {
  const queryClient = useQueryClient();
  const pages = useInfiniteQuery({
    queryKey: PAGES,
    queryFn: ({ pageParam }) => api.notifications(pageParam),
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (last: NotificationList) => last.next ?? undefined,
    retry: false,
  });
  const first = pages.data?.pages[0];
  const [fresh, setFresh] = useState<Set<number>>(() => new Set());
  const read = useMutation({
    mutationFn: (upTo: number) => api.readNotifications(upTo),
    onSettled: () => queryClient.invalidateQueries({ queryKey: NOTIFICATIONS }),
  });

  const newest = newestId(first);
  const unread = first?.unread ?? 0;
  useEffect(() => {
    if (!first || unread === 0 || newest === undefined) return;
    setFresh((seen) => {
      const next = new Set(seen);
      for (const n of first.notifications) if (!n.read_at) next.add(n.id);
      return next;
    });
    if (!read.isPending) read.mutate(newest);
    // Only when what is unread changes: the mutation itself is not news.
  }, [newest, unread]);

  const list = pages.data?.pages.flatMap((page) => page.notifications) ?? [];

  return (
    <div className="flex flex-col gap-6">
      <NowSection />
      <section aria-labelledby="earlier" className="flex flex-col gap-2">
        <h2 id="earlier" className="text-sm font-medium">
          Earlier
        </h2>
        {pages.isLoading ? (
          <Skeleton className="h-24 w-full" />
        ) : pages.isError ? (
          <p className="text-muted-foreground text-sm">
            Can't reach Niles: {pages.error.message}
          </p>
        ) : first && !first.kept ? (
          <p className="text-muted-foreground text-sm">
            Niles has no database, so it keeps no history of what it told you.
          </p>
        ) : list.length === 0 ? (
          <p className="text-muted-foreground text-sm">
            Nothing yet. When Niles tells you something, such as a plan that is ready or a run
            counted on it, it shows here.
          </p>
        ) : (
          <ul className="bg-card flex flex-col divide-y rounded-xl">
            {list.map((n) => (
              <Item key={n.id} notification={n} fresh={fresh.has(n.id)} />
            ))}
          </ul>
        )}
        {pages.hasNextPage && (
          <Button
            variant="ghost"
            size="sm"
            className="self-center"
            disabled={pages.isFetchingNextPage}
            onClick={() => void pages.fetchNextPage()}
          >
            Show older
          </Button>
        )}
      </section>
    </div>
  );
}

/**
 * Today's run, while there is a plan; the groceries, on the day they
 * come; and the offer to pair this phone, until one is paired. Each
 * says nothing when it has nothing to say, and with none to show there
 * is no section at all.
 */
function NowSection() {
  const queryClient = useQueryClient();
  const today = useQuery({
    queryKey: TRAINING_TODAY,
    queryFn: api.trainingToday,
    retry: false,
  });
  // Answers an error when nemlig.com is not switched on, and then there
  // is simply no card. Orders do not move often.
  const delivery = useQuery({
    queryKey: ["nemlig-next"],
    queryFn: api.nemligNext,
    retry: false,
    staleTime: 15 * 60_000,
  });
  // Asked once per visit and never retried: a 501 means there is no
  // console, which is a fine answer, and the card renders nothing until
  // an answer exists, so it can never appear and then withdraw.
  const phone = useQuery({
    queryKey: ["presence-device"],
    queryFn: api.deviceStatus,
    retry: false,
    staleTime: Infinity,
  });
  const pair = useMutation({
    mutationFn: api.pairDevice,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["presence-device"] }),
  });
  // Local today: the order's times are Danish, and so is the house.
  const day = new Date().toLocaleDateString("sv-SE");
  const any =
    Boolean(today.data?.plan) || arrivesToday(delivery.data, day) || shouldOffer(phone.data);
  if (!any) return null;
  return (
    <section aria-labelledby="now" className="flex flex-col gap-3">
      <h2 id="now" className="text-sm font-medium">
        Now
      </h2>
      <TodaysRunCard />
      <GroceryDeliveryCard order={delivery.data} today={day} />
      <PairPhoneCard device={phone.data} pairing={pair.isPending} onPair={() => pair.mutate()} />
    </section>
  );
}

const ICONS: Record<string, React.ComponentType<{ className?: string }>> = {
  artifact: FileText,
  shared: UsersRound,
  plan_changed: CalendarClock,
  workout: Footprints,
  approval: ShieldCheck,
};

function Item({ notification: n, fresh }: { notification: AppNotification; fresh: boolean }) {
  const Icon = ICONS[n.kind] ?? Bell;
  return (
    <li>
      <a
        href={hrefOf(n.url)}
        className={cn(
          "flex items-start gap-3 px-4 py-3 transition-colors",
          "hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none",
        )}
      >
        <Icon aria-hidden className="text-primary mt-0.5 size-5 shrink-0" />
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-2">
            <span className="min-w-0 flex-1 truncate text-sm font-medium">{n.title}</span>
            <time dateTime={n.created_at} className="text-muted-foreground shrink-0 text-xs">
              {relativeTime(n.created_at)}
            </time>
          </div>
          <p className="text-muted-foreground text-sm">{n.body}</p>
        </div>
        {fresh && (
          <span
            role="img"
            aria-label="New"
            className="bg-primary mt-2 size-2 shrink-0 rounded-full"
          />
        )}
      </a>
    </li>
  );
}

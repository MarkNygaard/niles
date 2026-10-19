import { useQuery } from "@tanstack/react-query";
import { Bell } from "lucide-react";
import { useDeviceStream } from "@/hooks/useDeviceStream";
import { api } from "@/lib/api";
import { badgeText } from "@/lib/approvals";
import { LATEST, bellCount } from "@/lib/notifications";
import { cn } from "@/lib/utils";

export interface NotificationBellProps {
  /** Whether the Notifications page is the one open. */
  current?: boolean;
}

/**
 * The bell by the avatar, which opens the Notifications page and counts
 * what is unread there. The stream keeps the count: a notice kept, or
 * read on another device, changes it without a reload.
 */
export function NotificationBell({ current = false }: NotificationBellProps) {
  useDeviceStream();
  const latest = useQuery({ queryKey: LATEST, queryFn: () => api.notifications(), retry: false });
  const unread = bellCount(latest.data);
  return (
    <a
      href="#/notifications"
      aria-label={unread > 0 ? `Notifications, ${unread} unread` : "Notifications"}
      aria-current={current ? "page" : undefined}
      className={cn(
        "relative flex size-9 shrink-0 items-center justify-center rounded-full transition-colors",
        "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
        current ? "text-foreground" : "text-muted-foreground hover:text-foreground",
      )}
    >
      <Bell aria-hidden className="size-5" />
      {unread > 0 && (
        <span
          aria-hidden
          data-slot="bell-badge"
          className="bg-primary text-primary-foreground absolute top-0.5 right-0 flex h-4 min-w-4 items-center justify-center rounded-full px-1 text-[10px] leading-none font-medium tabular-nums"
        >
          {badgeText(unread)}
        </span>
      )}
    </a>
  );
}

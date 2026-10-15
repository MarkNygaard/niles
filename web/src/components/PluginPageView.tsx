import { useQuery } from "@tanstack/react-query";
import { PluginSurface } from "@/components/PluginSurface";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useDeviceStream } from "@/hooks/useDeviceStream";
import { api } from "@/lib/api";

/**
 * A plugin's page from the menu: its surface, in a card like the rest
 * of the app's pages.
 *
 * The event stream is held here as well as on the house, so a
 * `ui_invalidated` frame reaches a page left open. A page whose plugin
 * is off, gone, or no longer declares it says so calmly in its place.
 */
export function PluginPageView({ plugin, page }: { plugin: string; page: string }) {
  useDeviceStream();
  const plugins = useQuery({ queryKey: ["plugins"], queryFn: api.plugins, retry: false });
  if (plugins.isPending) return <Skeleton className="h-64 w-full" />;
  const installed = plugins.data?.find((p) => p.id === plugin);
  const declared = installed?.enabled ? installed.menu.find((m) => m.id === page) : undefined;
  return (
    <Card>
      <CardContent>
        {declared ? (
          <PluginSurface
            plugin={plugin}
            surface={page}
            refreshEvery={declared.refresh_every}
          />
        ) : (
          <p data-surface="unavailable" className="text-muted-foreground py-2 text-sm">
            Not available right now
          </p>
        )}
      </CardContent>
    </Card>
  );
}

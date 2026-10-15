import { PluginSurface } from "@/components/PluginSurface";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { PlacedWidget } from "@/lib/plugins";

/** A plugin's widget on the front page: a card with its title. */
export function PluginWidgetCard({ placed }: { placed: PlacedWidget }) {
  return (
    <Card size="sm" data-widget={`${placed.plugin}.${placed.widget.id}`}>
      <CardHeader>
        <CardTitle>{placed.widget.title}</CardTitle>
      </CardHeader>
      <CardContent>
        <PluginSurface
          plugin={placed.plugin}
          surface={placed.widget.id}
          room={placed.room}
          refreshEvery={placed.widget.refresh_every}
        />
      </CardContent>
    </Card>
  );
}

/**
 * The widgets on a room's card. A room tile is a switch the size of a
 * thumb, so they are drawn where the room opens, under its lights:
 * a section each, titled, rather than a card inside a card.
 */
export function RoomWidgets({ widgets }: { widgets: PlacedWidget[] }) {
  return (
    <>
      {widgets.map((placed) => (
        <section
          key={`${placed.plugin}.${placed.widget.id}`}
          aria-label={placed.widget.title}
          data-widget={`${placed.plugin}.${placed.widget.id}`}
          className="flex flex-col gap-2 py-3"
        >
          <h3 className="text-muted-foreground text-xs font-medium tracking-wide uppercase">
            {placed.widget.title}
          </h3>
          <PluginSurface
            plugin={placed.plugin}
            surface={placed.widget.id}
            room={placed.room}
            refreshEvery={placed.widget.refresh_every}
          />
        </section>
      ))}
    </>
  );
}

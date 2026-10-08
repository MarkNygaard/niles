import { useEffect, useRef } from "react";
import L from "leaflet";
import "leaflet/dist/leaflet.css";

export interface HomeMapProps {
  latitude?: number;
  longitude?: number;
  disabled?: boolean;
  /** Where the pin was put: dragged, or a tap on the map. */
  onMove: (latitude: number, longitude: number) => void;
}

/** Close enough to say which house: five decimals is about a metre. */
function rounded(value: number): number {
  return Math.round(value * 1e5) / 1e5;
}

/** Null Island, which is what an unanswered location looks like. */
function isSet(latitude?: number, longitude?: number): boolean {
  return Boolean(latitude || longitude);
}

/**
 * Drawn rather than Leaflet's own marker, whose images a bundler cannot
 * find; and in the page's own colour, so it reads as the house.
 */
const PIN = L.divIcon({
  className: "",
  iconSize: [32, 32],
  iconAnchor: [16, 30],
  html: `<svg viewBox="0 0 24 24" width="32" height="32" style="color: var(--primary); filter: drop-shadow(0 1px 2px rgb(0 0 0 / 0.4))">
    <path fill="currentColor" stroke="white" stroke-width="1.5" d="M12 22s-7-6.4-7-12a7 7 0 0 1 14 0c0 5.6-7 12-7 12z"/>
    <circle cx="12" cy="10" r="2.6" fill="white"/>
  </svg>`,
});

/**
 * The house on a map, with a pin to put it in exactly the right place.
 *
 * OpenStreetMap's own tiles, the way Home Assistant does it: no key, no
 * account, and nobody's advertising business told where the house is.
 * The search finds the street; the pin is for the last few metres, and
 * for an address the map does not know the number of.
 */
export default function HomeMap({ latitude, longitude, disabled, onMove }: HomeMapProps) {
  const box = useRef<HTMLDivElement>(null);
  const map = useRef<L.Map | null>(null);
  const pin = useRef<L.Marker | null>(null);
  // The latest handler, without rebuilding the map when it changes.
  const moved = useRef(onMove);
  moved.current = onMove;

  useEffect(() => {
    if (!box.current) return;
    const m = L.map(box.current, {
      // A page scrolls with the wheel; a map that grabs it traps you.
      scrollWheelZoom: false,
      attributionControl: true,
    });
    L.tileLayer("https://tile.openstreetmap.org/{z}/{x}/{y}.png", {
      maxZoom: 19,
      attribution:
        '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors',
    }).addTo(m);

    const marker = L.marker([0, 0], { draggable: true, icon: PIN, keyboard: true, title: "Home" });
    marker.on("dragend", () => {
      const at = marker.getLatLng();
      moved.current(rounded(at.lat), rounded(at.lng));
    });
    // A tap puts the pin there, which is the only way to place one on a
    // map that has none yet.
    m.on("click", (event: L.LeafletMouseEvent) => {
      if (!marker.dragging?.enabled()) return;
      marker.setLatLng(event.latlng).addTo(m);
      moved.current(rounded(event.latlng.lat), rounded(event.latlng.lng));
    });

    map.current = m;
    pin.current = marker;
    return () => {
      m.remove();
      map.current = null;
      pin.current = null;
    };
  }, []);

  // Follow the config: the search moves the house, and so does typing
  // the numbers in.
  useEffect(() => {
    const m = map.current;
    const marker = pin.current;
    if (!m || !marker) return;
    if (isSet(latitude, longitude)) {
      const at = L.latLng(latitude!, longitude!);
      marker.setLatLng(at).addTo(m);
      m.setView(at, Math.max(m.getZoom() || 0, 17));
    } else {
      marker.remove();
      m.setView([30, 0], 2);
    }
  }, [latitude, longitude]);

  useEffect(() => {
    const marker = pin.current;
    if (!marker?.dragging) return;
    if (disabled) marker.dragging.disable();
    else marker.dragging.enable();
  }, [disabled]);

  return (
    <div
      ref={box}
      role="application"
      aria-label="Map of the house's location"
      // Its own stacking context: Leaflet's panes sit at z-index 400 and
      // up, and would otherwise draw over the tab bar as the page scrolls.
      className="bg-muted isolate h-64 w-full overflow-hidden rounded-xl dark:[&_.leaflet-tile]:brightness-[0.8]"
    />
  );
}

import { Button } from "@/components/ui/button";
import { Smartphone } from "lucide-react";
import type { DeviceView } from "@/lib/api";

export interface PairPhoneCardProps {
  device?: DeviceView;
  pairing?: boolean;
  onPair: () => void;
}

/**
 * Whether there is anything to offer.
 *
 * Every condition is a reason the button would not work, and each is
 * checked rather than assumed. No console to ask; a request through the
 * tunnel, whose address belongs to Cloudflare and not to a phone; a
 * console that cannot see this device; nobody signed in to pair it to;
 * or a phone already paired.
 *
 * `undefined` — the answer has not arrived — is deliberately *not* a
 * reason to show it. A dashboard that offers this and then withdraws it
 * a moment later has lied about the state of the house, and this is the
 * one thing on the page that appears unprompted.
 */
export function shouldOffer(device: DeviceView | undefined): boolean {
  if (!device) return false;
  return (
    device.available &&
    device.on_home_network &&
    device.signed_in &&
    !device.paired &&
    device.mac !== null
  );
}

/**
 * "This is my phone."
 *
 * Presence from the thermostats is minutes late: coming home took ten
 * minutes to register once, because tado is polled every five. A phone
 * joins the Wi-Fi the moment it is in range, so the network knows
 * before the door does — but only if Niles can tell which device on it
 * is yours.
 *
 * Nobody should type a MAC address to answer that. The request itself
 * carries the answer: it arrives from an address the console can name,
 * so the phone identifies itself by asking.
 */
export function PairPhoneCard({ device, pairing, onPair }: PairPhoneCardProps) {
  if (!shouldOffer(device)) return null;

  return (
    <div className="bg-muted/60 flex items-center justify-between gap-3 rounded-2xl px-4 py-3">
      <span className="flex min-w-0 items-center gap-3">
        <Smartphone aria-hidden className="text-muted-foreground size-5 shrink-0" />
        <span className="min-w-0">
          <span className="block text-sm font-medium">
            Is this {device?.name ?? "phone"} yours?
          </span>
          <span className="text-muted-foreground block text-xs">
            Pair it and Niles knows you are home as soon as you are in range,
            rather than when the thermostats next report.
          </span>
        </span>
      </span>
      <Button size="sm" disabled={pairing} onClick={onPair}>
        This is my phone
      </Button>
    </div>
  );
}

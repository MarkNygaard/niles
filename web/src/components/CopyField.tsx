import { useEffect, useState } from "react";
import { Check, Copy } from "lucide-react";
import { Button } from "@/components/ui/button";

/** A value to copy somewhere else, with a copy button. Shown in full:
    it is meant to be pasted into another system's settings. */
export function CopyField({ label, value }: { label: string; value: string }) {
  const [copied, setCopied] = useState(false);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 2000);
    return () => clearTimeout(timer);
  }, [copied]);

  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-2">
        <code
          aria-label={label}
          className="bg-muted min-w-0 flex-1 rounded-md px-3 py-2 font-mono text-xs break-all select-all"
        >
          {value}
        </code>
        <Button
          variant="ghost"
          size="icon"
          aria-label={`Copy the ${label.toLowerCase()}`}
          onClick={async () => {
            setFailed(false);
            try {
              await navigator.clipboard.writeText(value);
              setCopied(true);
            } catch {
              setFailed(true);
            }
          }}
        >
          {copied ? <Check /> : <Copy />}
        </Button>
      </div>
      <span className="text-muted-foreground text-xs" role="status">
        {copied ? "Copied" : failed ? "Could not copy; select it and copy it by hand" : ""}
      </span>
    </div>
  );
}

import { useState } from "react";
import { Input } from "@/components/ui/input";

/** Writes when you leave it, so half a name is never saved. */
export function CommitField({
  label,
  value,
  placeholder,
  disabled,
  onCommit,
}: {
  label: string;
  value: string;
  placeholder: string;
  disabled?: boolean;
  onCommit: (value: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  const [seen, setSeen] = useState(value);
  if (seen !== value) {
    setSeen(value);
    setDraft(value);
  }

  return (
    <label className="flex min-w-0 flex-1 flex-col gap-1">
      <span className="text-muted-foreground text-xs">{label}</span>
      <Input
        value={draft}
        aria-label={label}
        placeholder={placeholder}
        disabled={disabled}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => {
          if (draft.trim() && draft !== value) onCommit(draft.trim());
          else setDraft(value);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
          if (e.key === "Escape") setDraft(value);
        }}
      />
    </label>
  );
}

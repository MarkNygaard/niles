import { useState } from "react";
import { cn } from "@/lib/utils";

export interface AvatarProps {
  /** The signed-in address, or undefined when sign-in is off. */
  email?: string;
  /** Their GitHub avatar, when there is one. */
  avatarUrl?: string;
  className?: string;
}

/** Your picture, or your initials when there is no picture to show. */
export function Avatar({ email, avatarUrl, className }: AvatarProps) {
  // The picture comes from GitHub, so it can be slow, blocked by a
  // content blocker, or simply gone. Any of those falls back to the
  // letters rather than leaving a hole.
  const [broken, setBroken] = useState(false);
  const picture = avatarUrl && !broken;

  return (
    <span
      aria-hidden
      className={cn(
        "flex shrink-0 items-center justify-center overflow-hidden rounded-full font-medium",
        !picture && "bg-primary text-primary-foreground",
        className,
      )}
    >
      {picture ? (
        <img
          src={avatarUrl}
          alt=""
          // Nothing about this house travels to GitHub with the request
          // for a picture.
          referrerPolicy="no-referrer"
          className="size-full object-cover"
          onError={() => setBroken(true)}
        />
      ) : (
        initials(email)
      )}
    </span>
  );
}

/**
 * What to draw when there is no picture.
 *
 * The avatar comes from GitHub, addressed by the numeric account id the
 * session carries — not from Gravatar, which would mean handing a third
 * party the hash of a household member's address. When it is missing,
 * slow or blocked, two letters are better than a hole.
 */
export function initials(email?: string): string {
  if (!email) return "·";
  const [local] = email.split("@");
  const parts = local.split(/[._-]+/).filter(Boolean);
  if (parts.length >= 2) {
    return (parts[0][0] + parts[1][0]).toUpperCase();
  }
  return (local.slice(0, 2) || "·").toUpperCase();
}

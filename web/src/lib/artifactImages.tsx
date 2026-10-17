import { createContext, useContext, useMemo } from "react";
import type { ReactNode } from "react";
import { artifactImageUrl } from "@/lib/api";
import type { ArtifactImage } from "@/lib/api";
import { cn } from "@/lib/utils";

/**
 * The pictures of the artifact open, for its kind's view to show.
 *
 * Content names a picture as `niles-image:12`: one Niles keeps, fetched
 * from Niles (`artifactImageUrl`). Nothing else is ever an image here.
 * A picture's web address in the content would be a request from this
 * phone to that site each time the artifact is looked at, so it is
 * never drawn; and a name of one the artifact no longer keeps shows
 * nothing. See crates/niles-artifacts/src/images.rs.
 */
export const IMAGE_SCHEME = "niles-image:";

/** The id a whole `niles-image:12` names. */
export function imageId(name: string | undefined): number | undefined {
  const m = /^\s*niles-image:(\d{1,15})\s*$/.exec(name ?? "");
  const id = m ? Number(m[1]) : NaN;
  return id > 0 ? id : undefined;
}

interface Kept {
  artifact: number;
  images: ReadonlyMap<number, ArtifactImage>;
}

const Pictures = createContext<Kept | undefined>(undefined);

export function ArtifactImagesProvider({
  artifact,
  images,
  children,
}: {
  artifact: number;
  images: ArtifactImage[] | undefined;
  children: ReactNode;
}) {
  const value = useMemo(
    () => ({ artifact, images: new Map((images ?? []).map((i) => [i.id, i])) }),
    [artifact, images],
  );
  return <Pictures.Provider value={value}>{children}</Pictures.Provider>;
}

/** The picture `name` names, with where to fetch it, when the artifact
    open keeps it. */
export function useArtifactImage(
  name: string | undefined,
): { image: ArtifactImage; src: string } | undefined {
  const kept = useContext(Pictures);
  const id = imageId(name);
  const image = id === undefined ? undefined : kept?.images.get(id);
  return image && kept ? { image, src: artifactImageUrl(kept.artifact, image.id) } : undefined;
}

/** The host of an https address, to credit a picture's page by. */
export function sourceHost(source: string | null | undefined): string | undefined {
  if (!source) return undefined;
  try {
    const url = new URL(source);
    return url.protocol === "https:" ? url.hostname.replace(/^www\./, "") : undefined;
  } catch {
    return undefined;
  }
}

/**
 * A kept picture with its credit under it, small: who made it, and the
 * page it is from. Nothing for a name the artifact does not keep.
 */
export function ArtifactPicture({
  name,
  alt,
  className,
}: {
  name: string | undefined;
  /** In place of the picture's own alt, when the content gives one. */
  alt?: string;
  className?: string;
}) {
  const kept = useArtifactImage(name);
  if (!kept) return null;
  const { image, src } = kept;
  const host = sourceHost(image.source);
  return (
    <figure className={cn("flex flex-col gap-1", className)}>
      <img
        src={src}
        alt={alt || image.alt}
        width={image.width}
        height={image.height}
        loading="lazy"
        decoding="async"
        referrerPolicy="no-referrer"
        className="bg-muted h-auto w-full rounded-lg object-cover"
      />
      {(image.credit || host) && (
        <figcaption className="text-muted-foreground text-[11px] leading-snug">
          {image.credit}
          {image.credit && host && " · "}
          {host && image.source && (
            <a
              href={image.source}
              target="_blank"
              rel="noopener noreferrer nofollow"
              className="underline-offset-2 hover:underline"
            >
              {host}
            </a>
          )}
        </figcaption>
      )}
    </figure>
  );
}

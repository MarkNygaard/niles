/**
 * Add-ons other people published, from the add-on library. Listed
 * first, Unreviewed folded away; Install is the same preview as a
 * pasted address.
 */

import { Fragment, useState } from "react";
import { ChevronRight } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { GroupList } from "@/components/GroupList";
import { PermissionList, PluginLogo } from "@/components/PluginPreview";
import { day } from "@/lib/plugins";
import {
  countLine,
  entryKey,
  installedFrom,
  shelvesOf,
  starsLine,
  useLibrary,
} from "@/lib/library";
import type { LibraryEntry, PluginLibrary, PluginSummary } from "@/lib/api";
import { cn } from "@/lib/utils";

export interface BrowseTabProps {
  query: string;
  plugins: PluginSummary[];
  disabled?: boolean;
  onInstall: (repo: string) => void;
  onOpenInstalled: (id: string) => void;
  onAddByAddress: () => void;
}

export function BrowseTab({
  query,
  plugins,
  disabled,
  onInstall,
  onOpenInstalled,
  onAddByAddress,
}: BrowseTabProps) {
  const library = useLibrary();
  const [open, setOpen] = useState<string | null>(null);

  if (library.status === "pending") {
    return (
      <p className="text-muted-foreground px-1 text-sm">Loading the add-on library…</p>
    );
  }
  if (library.status === "error") {
    return (
      <div className="flex flex-col items-start gap-2">
        <p className="text-sm">Couldn't load the add-on library.</p>
        <p className="text-muted-foreground text-sm">{library.error.message}</p>
        <p className="text-muted-foreground text-sm">
          You can still install an add-on by its address.
        </p>
        <div className="flex gap-2">
          <Button size="sm" variant="outline" onClick={() => void library.refetch()}>
            Try again
          </Button>
          <Button size="sm" onClick={onAddByAddress}>
            Add-on from GitHub
          </Button>
        </div>
      </div>
    );
  }
  const data = library.data;
  const openEntry = open === null ? null : (data.addons.find((e) => entryKey(e) === open) ?? null);
  return (
    <>
      {data.addons.length === 0 && query.trim() === "" ? (
        <p className="text-muted-foreground px-1 text-sm">
          Nothing is in the add-on library yet.
        </p>
      ) : (
        <Shelves
          library={data}
          query={query}
          plugins={plugins}
          disabled={disabled}
          onInstall={onInstall}
          onOpenDetails={setOpen}
        />
      )}
      <LibraryDetails
        entry={openEntry}
        cannot={data.cannot}
        plugins={plugins}
        disabled={disabled}
        onInstall={onInstall}
        onOpenInstalled={onOpenInstalled}
        onClose={() => setOpen(null)}
      />
    </>
  );
}

/** Listed first; Unreviewed folded under its own toggle. Removed ones
 * never appear. */
function Shelves({
  library,
  query,
  plugins,
  disabled,
  onInstall,
  onOpenDetails,
}: {
  library: PluginLibrary;
  query: string;
  plugins: PluginSummary[];
  disabled?: boolean;
  onInstall: (address: string) => void;
  onOpenDetails: (key: string) => void;
}) {
  const [showUnreviewed, setShowUnreviewed] = useState(false);
  const { listed, unreviewed } = shelvesOf(library, query);
  const searching = query.trim() !== "";
  const bothEmpty = searching && listed.length === 0 && unreviewed.length === 0;
  const row = (entry: LibraryEntry) => (
    <LibraryRow
      key={entryKey(entry)}
      entry={entry}
      plugins={plugins}
      disabled={disabled}
      onInstall={onInstall}
      onDetails={() => onOpenDetails(entryKey(entry))}
    />
  );
  return (
    <>
      <p className="text-muted-foreground px-1 text-xs">
        Add-ons other people published on GitHub. Each one still shows what it may do before it
        is installed.
      </p>
      <GroupList
        id="browse-group-listed"
        label="Listed"
        footer={
          <p className="text-muted-foreground px-3 text-xs">
            Checked when they were listed: the release installs, and someone read what it asks
            for. Nobody audited the code.
          </p>
        }
      >
        {listed.map(row)}
        {listed.length === 0 && !bothEmpty && (
          <p className="text-muted-foreground px-3 py-3 text-sm">
            {searching
              ? `No listed add-on matches “${query.trim()}”.`
              : "Nothing is listed yet."}
          </p>
        )}
      </GroupList>
      {bothEmpty && (
        <p className="text-muted-foreground px-1 text-sm">
          Nothing in the add-on library matches “{query.trim()}”.
        </p>
      )}
      {unreviewed.length > 0 && (
        <button
          type="button"
          aria-expanded={showUnreviewed}
          onClick={() => setShowUnreviewed(!showUnreviewed)}
          className="text-muted-foreground hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 flex items-center gap-1 self-start rounded-md px-1 text-left text-sm focus-visible:outline-none"
        >
          Unreviewed ({unreviewed.length})
          <ChevronRight
            aria-hidden
            className={cn("size-4 shrink-0 transition-transform", showUnreviewed && "rotate-90")}
          />
        </button>
      )}
      {showUnreviewed && unreviewed.length > 0 && (
        <>
          <p className="text-muted-foreground px-1 text-xs">
            Tagged niles-addon by their authors on GitHub. Nobody has checked them.
          </p>
          <GroupList id="browse-group-unreviewed" label="Unreviewed">
            {unreviewed.map(row)}
          </GroupList>
        </>
      )}
      {library.generated_at && (
        <p className="text-muted-foreground px-1 text-xs">
          The library was updated {day(library.generated_at)}.
        </p>
      )}
    </>
  );
}

/** One add-on: its mark, name, a line about it, and what can be done
 * with it. */
function LibraryRow({
  entry,
  plugins,
  disabled,
  onInstall,
  onDetails,
}: {
  entry: LibraryEntry;
  plugins: PluginSummary[];
  disabled?: boolean;
  onInstall: (repo: string) => void;
  onDetails: () => void;
}) {
  const installed = installedFrom(entry, plugins);
  const when = day(entry.published_at);
  const bits: React.ReactNode[] = [
    <a
      key="owner"
      href={`https://github.com/${encodeURIComponent(entry.owner)}`}
      target="_blank"
      rel="noopener noreferrer"
      className="hover:underline"
    >
      {entry.owner}
    </a>,
    ...(entry.version ? [<span key="v">v{entry.version}</span>] : []),
    ...(when ? [<span key="at">{when}</span>] : []),
    <span key="count">{countLine(entry)}</span>,
    <span key="stars">{starsLine(entry.stars, entry.shares_repo)}</span>,
  ];
  return (
    <div className="flex min-h-12 items-center gap-3 py-2 pr-1.5 pl-3">
      <PluginLogo svg={entry.logo} />
      <div className="min-w-0 flex-1">
        <button
          type="button"
          onClick={onDetails}
          className="block max-w-full truncate text-left text-sm font-medium hover:underline focus-visible:underline focus-visible:outline-none"
        >
          {entry.name}
        </button>
        <p className="text-muted-foreground truncate text-xs">{entry.description}</p>
        <p className="text-muted-foreground truncate text-xs">
          {bits.map((bit, i) => (
            <Fragment key={i}>
              {i > 0 && " · "}
              {bit}
            </Fragment>
          ))}
        </p>
      </div>
      {installed ? (
        <Badge variant="outline">Installed</Badge>
      ) : entry.blocked ? (
        <Badge variant="outline">
          {entry.needs_newer_niles ? "Needs a newer Niles" : "Can't install"}
        </Badge>
      ) : (
        <Button
          variant="link"
          size="sm"
          disabled={disabled}
          aria-label={`Install ${entry.name}`}
          onClick={() => onInstall(entry.address)}
        >
          Install
        </Button>
      )}
    </div>
  );
}

/** One add-on's details: what it is, its numbers, what it asks for in
 * the approval screen's words, and Install. */
function LibraryDetails({
  entry,
  cannot,
  plugins,
  disabled,
  onInstall,
  onOpenInstalled,
  onClose,
}: {
  entry: LibraryEntry | null;
  cannot: string;
  plugins: PluginSummary[];
  disabled?: boolean;
  onInstall: (repo: string) => void;
  onOpenInstalled: (id: string) => void;
  onClose: () => void;
}) {
  if (!entry) return null;
  const installed = installedFrom(entry, plugins);
  const when = day(entry.published_at);
  return (
    <Dialog
      open={entry !== null}
      onOpenChange={(next: boolean) => {
        if (!next) onClose();
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <div className="flex min-w-0 flex-col gap-1">
            <DialogTitle className="flex items-center gap-2">
              <PluginLogo svg={entry.logo} />
              <span className="truncate">{entry.name}</span>
              {entry.version && (
                <span className="text-muted-foreground font-normal">v{entry.version}</span>
              )}
            </DialogTitle>
            <DialogDescription>{entry.description}</DialogDescription>
          </div>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-4 text-sm">
          {entry.standing === "listed" ? (
            <p>
              <span className="font-medium">Listed.</span> It passed the add-on library's checks
              when it was listed, and someone read what it asks for. Nobody audited its code.
            </p>
          ) : (
            <p>
              <span className="font-medium">Unreviewed.</span> Its author tagged it for Niles on
              GitHub. Nobody has checked it.
            </p>
          )}
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
            <dt className="text-muted-foreground">By</dt>
            <dd>
              <a
                href={`https://github.com/${encodeURIComponent(entry.owner)}`}
                target="_blank"
                rel="noopener noreferrer"
                className="hover:underline"
              >
                {entry.owner}
              </a>
            </dd>
            <dt className="text-muted-foreground">Latest</dt>
            <dd>
              {[entry.version ? `v${entry.version}` : null, when].filter(Boolean).join(", ") ||
                "—"}
            </dd>
            <dt className="text-muted-foreground">
              {entry.installs !== null ? "Installs" : "Downloads"}
            </dt>
            <dd>
              {(entry.installs ?? entry.downloads).toLocaleString("en")}
              {entry.installs === null && (
                <span className="text-muted-foreground"> (previews and updates count too)</span>
              )}
            </dd>
            <dt className="text-muted-foreground">Stars</dt>
            <dd>
              {entry.stars.toLocaleString("en")}
              {entry.shares_repo && (
                <span className="text-muted-foreground">
                  {" "}
                  (its repo's, shared with the other add-ons in it)
                </span>
              )}
            </dd>
            <dt className="text-muted-foreground">On GitHub</dt>
            <dd className="flex gap-3">
              <a
                href={entry.repo}
                target="_blank"
                rel="noopener noreferrer"
                className="underline underline-offset-2"
              >
                Repository
              </a>
              <a
                href={entry.releases}
                target="_blank"
                rel="noopener noreferrer"
                className="underline underline-offset-2"
              >
                Releases
              </a>
            </dd>
          </dl>
          {entry.readme && (
            <p className="text-muted-foreground whitespace-pre-line">{entry.readme}</p>
          )}
          <section aria-label="Permissions" className="flex flex-col gap-1">
            <p className="font-medium">It will be able to</p>
            {entry.permissions ? (
              <PermissionList permissions={entry.permissions} cannot={cannot} />
            ) : (
              <p className="text-muted-foreground">This Niles can't read what it asks for.</p>
            )}
          </section>
          {entry.blocked && (
            <p className="text-destructive">
              {entry.needs_newer_niles ? "Needs a newer Niles." : "Can't be installed here."}{" "}
              {entry.blocked}
            </p>
          )}
          <div className="flex gap-2">
            {installed ? (
              <Button
                onClick={() => {
                  onClose();
                  onOpenInstalled(installed.id);
                }}
              >
                Open
              </Button>
            ) : (
              <Button
                disabled={disabled || entry.blocked !== null}
                onClick={() => {
                  onClose();
                  onInstall(entry.address);
                }}
              >
                Install
              </Button>
            )}
            <Button variant="ghost" onClick={onClose}>
              Close
            </Button>
          </div>
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}

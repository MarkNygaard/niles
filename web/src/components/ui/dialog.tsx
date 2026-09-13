"use client"

import { Dialog as DialogPrimitive } from "@base-ui/react/dialog"

import { cn } from "@/lib/utils"

function Dialog(props: DialogPrimitive.Root.Props) {
  return <DialogPrimitive.Root {...props} />
}

function DialogTrigger({ className, ...props }: DialogPrimitive.Trigger.Props) {
  return (
    <DialogPrimitive.Trigger
      data-slot="dialog-trigger"
      className={cn(className)}
      {...props}
    />
  )
}

function DialogClose({ className, ...props }: DialogPrimitive.Close.Props) {
  return (
    <DialogPrimitive.Close
      data-slot="dialog-close"
      className={cn(className)}
      {...props}
    />
  )
}

/**
 * A sheet on a phone, a dialog on a desktop.
 *
 * One component rather than two: the difference is where it sits and
 * which corners are round, which is a media query, not a component.
 */
function DialogContent({ className, ...props }: DialogPrimitive.Popup.Props) {
  return (
    <DialogPrimitive.Portal>
      <DialogPrimitive.Backdrop
        data-slot="dialog-backdrop"
        className="fixed inset-0 z-50 bg-black/50 transition-opacity duration-150 data-ending-style:opacity-0 data-starting-style:opacity-0"
      />
      <DialogPrimitive.Popup
        data-slot="dialog-content"
        className={cn(
          "fixed z-50 flex flex-col bg-popover text-popover-foreground shadow-xl ring-1 ring-foreground/10 outline-none",
          // Phone: rises from the bottom edge, never taller than the
          // screen, and clear of the home indicator.
          "inset-x-0 bottom-0 max-h-[85dvh] rounded-t-xl pb-[env(safe-area-inset-bottom)]",
          "transition-transform duration-200 data-ending-style:translate-y-full data-starting-style:translate-y-full",
          // Desktop: centred, and the slide-up becomes a fade so it
          // doesn't fly in from off-screen.
          "sm:inset-x-auto sm:top-1/2 sm:bottom-auto sm:left-1/2 sm:max-h-[85vh] sm:w-[28rem] sm:-translate-x-1/2 sm:-translate-y-1/2 sm:rounded-xl sm:pb-0",
          "sm:transition-opacity sm:data-ending-style:translate-y-[-50%] sm:data-ending-style:opacity-0 sm:data-starting-style:translate-y-[-50%] sm:data-starting-style:opacity-0",
          className
        )}
        {...props}
      />
    </DialogPrimitive.Portal>
  )
}

function DialogHeader({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="dialog-header"
      className={cn(
        "flex items-start justify-between gap-3 border-b border-border px-4 py-3",
        className
      )}
      {...props}
    />
  )
}

function DialogBody({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="dialog-body"
      className={cn("min-h-0 flex-1 overflow-y-auto px-4 py-3", className)}
      {...props}
    />
  )
}

function DialogTitle({ className, ...props }: DialogPrimitive.Title.Props) {
  return (
    <DialogPrimitive.Title
      data-slot="dialog-title"
      className={cn("font-heading text-base leading-snug font-medium", className)}
      {...props}
    />
  )
}

function DialogDescription({
  className,
  ...props
}: DialogPrimitive.Description.Props) {
  return (
    <DialogPrimitive.Description
      data-slot="dialog-description"
      className={cn("text-sm text-muted-foreground", className)}
      {...props}
    />
  )
}

export {
  Dialog,
  DialogBody,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
}

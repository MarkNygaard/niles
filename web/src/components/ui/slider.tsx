import { Slider as SliderPrimitive } from "@base-ui/react/slider"

import { cn } from "@/lib/utils"

/**
 * The registry component, with three additions, all of them props:
 *
 * - `thumbLabel` / `thumbValueText`, because Base UI puts the range
 *   input inside the thumb, so a label on the root labels nothing and
 *   the registry version generates its thumbs internally.
 * - `trackClassName` / `indicatorClassName`, so the colour-temperature
 *   slider can paint the track with the whites it spans.
 *
 * The indicator is deliberately not `bg-primary`: the brand green on a
 * brightness fill says nothing about brightness.
 */
function Slider({
  className,
  defaultValue,
  value,
  min = 0,
  max = 100,
  thumbLabel,
  thumbValueText,
  trackClassName,
  indicatorClassName,
  ...props
}: SliderPrimitive.Root.Props<number> & {
  thumbLabel?: string
  thumbValueText?: string
  trackClassName?: string
  indicatorClassName?: string
}) {
  // The registry falls back to `[min, max]` — two thumbs — whenever it
  // isn't handed an array, which gives a single-value slider a second
  // thumb sitting on top of the first. A number is one thumb.
  const thumbs = Array.isArray(value)
    ? value.length
    : Array.isArray(defaultValue)
      ? defaultValue.length
      : value === undefined && defaultValue === undefined
        ? 2
        : 1

  return (
    <SliderPrimitive.Root
      className={cn("data-horizontal:w-full data-vertical:h-full", className)}
      data-slot="slider"
      defaultValue={defaultValue}
      value={value}
      min={min}
      max={max}
      // The registry asks for `edge`, which holds the thumb hidden
      // until it has measured the control. That needs a ResizeObserver,
      // so it can never resolve under test and could not be verified
      // here. `center` is Base UI's own default, needs no measurement,
      // and only differs in how the thumb sits at the two extremes.
      thumbAlignment="center"
      {...props}
    >
      <SliderPrimitive.Control className="relative flex w-full touch-none items-center py-2 select-none data-disabled:opacity-50 data-vertical:h-full data-vertical:min-h-40 data-vertical:w-auto data-vertical:flex-col">
        <SliderPrimitive.Track
          data-slot="slider-track"
          className={cn(
            "relative grow overflow-hidden rounded-full bg-muted select-none data-horizontal:h-1.5 data-horizontal:w-full data-vertical:h-full data-vertical:w-1",
            trackClassName
          )}
        >
          <SliderPrimitive.Indicator
            data-slot="slider-range"
            className={cn(
              "bg-foreground select-none data-horizontal:h-full data-vertical:w-full",
              indicatorClassName
            )}
          />
        </SliderPrimitive.Track>
        {Array.from({ length: thumbs }, (_, index) => (
          <SliderPrimitive.Thumb
            data-slot="slider-thumb"
            key={index}
            aria-label={thumbLabel}
            aria-valuetext={thumbValueText}
            className="relative block size-4 shrink-0 rounded-full border border-ring bg-background shadow ring-ring/50 transition-[color,box-shadow] select-none after:absolute after:-inset-2 hover:ring-3 focus-visible:ring-3 focus-visible:outline-hidden active:ring-3 disabled:pointer-events-none disabled:opacity-50"
          />
        ))}
      </SliderPrimitive.Control>
    </SliderPrimitive.Root>
  )
}

export { Slider }

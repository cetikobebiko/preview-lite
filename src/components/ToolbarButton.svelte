<script>
  import * as Tooltip from "$lib/components/ui/tooltip";
  import { buttonVariants } from "$lib/components/ui/button";
  import { cn } from "$lib/utils";

  // Icon-only toolbar button: the label is both its accessible name and its tooltip.
  let { label, hint = "", pressed = undefined, disabled = false, onclick, class: className = "", children } = $props();
</script>

<Tooltip.Root>
  <Tooltip.Trigger
    class={cn(buttonVariants({ variant: "ghost", size: "icon-sm" }), "text-foreground/75 hover:text-foreground aria-pressed:bg-accent aria-pressed:text-foreground", className)}
    aria-label={label}
    aria-pressed={pressed}
    {disabled}
    {onclick}
  >
    {@render children?.()}
  </Tooltip.Trigger>
  <Tooltip.Content side="bottom" sideOffset={6}>{hint || label}</Tooltip.Content>
</Tooltip.Root>

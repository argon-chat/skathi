<script setup>
// TIER 3 — portal + floating components. These are the ones that reach deepest
// into the DOM: Teleport targets, focus traps, dismissable layers, and
// @floating-ui positioning driven by getBoundingClientRect.
//
// The ONE piece of app-level configuration this tier needs is reka-ui's own
// documented `ConfigProvider :teleport-to`: reka's <Teleport> defaults to the
// STRING "body", and Vue's Teleport resolves a string target through
// `nodeOps.querySelector`, which our renderer does not implement. Handing it a
// real element uses a public reka API instead of touching any component.
import { ref } from "vue";
import { ConfigProvider } from "reka-ui";
import Section from "./Section.vue";
import { Button } from "../components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger } from "../components/ui/dialog";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "../components/ui/tooltip";
import { Popover, PopoverContent, PopoverTrigger } from "../components/ui/popover";
import { Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectTrigger, SelectValue } from "../components/ui/select";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from "../components/ui/dropdown-menu";

const portal = ref(null);
const fruit = ref("apple");
</script>

<template>
  <div class="flex flex-col gap-4">
    <ConfigProvider :teleport-to="portal">
      <!--
        The Teleport target, declared FIRST so the template ref is populated
        before anything that might portal into it mounts. In a browser this is
        <body>; here it is an ordinary element the renderer already owns, so
        portalled content lands in the real tree instead of a detached one.
      -->
      <div ref="portal" class="flex flex-col" />

      <Section title="Dialog" note="Teleport + FocusScope + DismissableLayer">
        <Dialog>
          <DialogTrigger as-child>
            <Button variant="outline">Open dialog</Button>
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Portalled content</DialogTitle>
              <DialogDescription>Rendered through Vue's Teleport into a detached container.</DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button size="sm">Confirm</Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </Section>

      <Section title="Tooltip / Popover" note="@floating-ui positioning">
        <div class="flex flex-row gap-2">
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger as-child>
                <Button variant="outline" size="sm">Hover me</Button>
              </TooltipTrigger>
              <TooltipContent>Tooltip body</TooltipContent>
            </Tooltip>
          </TooltipProvider>

          <Popover>
            <PopoverTrigger as-child>
              <Button variant="outline" size="sm">Popover</Button>
            </PopoverTrigger>
            <PopoverContent>
              <div class="text-sm">Popover body</div>
            </PopoverContent>
          </Popover>
        </div>
      </Section>

      <Section title="Select" note="Collection + Teleport + typeahead">
        <Select v-model="fruit">
          <SelectTrigger>
            <SelectValue placeholder="Pick a fruit" />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              <SelectLabel>Fruits</SelectLabel>
              <SelectItem value="apple">Apple</SelectItem>
              <SelectItem value="banana">Banana</SelectItem>
            </SelectGroup>
          </SelectContent>
        </Select>
        <div class="text-xs text-muted-foreground">selected: {{ fruit }}</div>
      </Section>

      <Section title="DropdownMenu" note="Menu + RovingFocus + focus guards">
        <DropdownMenu>
          <DropdownMenuTrigger as-child>
            <Button variant="outline" size="sm">Open menu</Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent>
            <DropdownMenuLabel>Account</DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuItem>Profile</DropdownMenuItem>
            <DropdownMenuItem>Settings</DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </Section>
    </ConfigProvider>
  </div>
</template>

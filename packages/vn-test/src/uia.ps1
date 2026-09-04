# Walks a process's UI Automation tree and prints it as JSON, one flat row per
# node with its depth.
#
# This is the OUTSIDE view of `crates/vn-tree/src/aria.rs`: it asks Windows what
# a screen reader would see, through the same client API Narrator and every
# other AT uses. Nothing here touches the window — no activation, no focus, no
# synthetic input — so a suite can read the accessibility tree of an app the
# user is not looking at without stealing their cursor.
#
# Attaching a UIA client is also what ACTIVATES AccessKit inside the process:
# gpui builds no accessibility tree until something asks for one, so the first
# walk of a freshly launched window can come back with the tree still empty.
# Callers retry.

param(
  [Parameter(Mandatory=$true)][int]$ProcessId,
  [int]$MaxDepth = 40,
  [int]$MaxNodes = 4000
)

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$root = [System.Windows.Automation.AutomationElement]::RootElement
$cond = New-Object System.Windows.Automation.PropertyCondition(
  [System.Windows.Automation.AutomationElement]::ProcessIdProperty, $ProcessId)
$windows = $root.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)

$walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
$out = New-Object System.Collections.Generic.List[object]
$count = 0

function Fin($v) {
  # An offscreen element reports an infinite rect; JSON has no infinity.
  if ([double]::IsInfinity($v) -or [double]::IsNaN($v)) { return -1 }
  return [int]$v
}

function Emit($el, $depth) {
  $script:count++
  $c = $el.Current
  $row = [ordered]@{
    depth     = $depth
    type      = $c.ControlType.ProgrammaticName -replace '^ControlType\.', ''
    name      = $c.Name
    autoId    = $c.AutomationId
    enabled   = $c.IsEnabled
    offscreen = $c.IsOffscreen
    x = (Fin $c.BoundingRectangle.X)
    y = (Fin $c.BoundingRectangle.Y)
    w = (Fin $c.BoundingRectangle.Width)
    h = (Fin $c.BoundingRectangle.Height)
  }
  $p = $null
  if ($el.TryGetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern, [ref]$p)) {
    $row.toggle = $p.Current.ToggleState.ToString()
  }
  $p = $null
  if ($el.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$p)) {
    $row.selected = $p.Current.IsSelected
  }
  $p = $null
  if ($el.TryGetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern, [ref]$p)) {
    $row.expand = $p.Current.ExpandCollapseState.ToString()
  }
  $p = $null
  if ($el.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$p)) {
    $row.value = $p.Current.Value
    $row.readonly = $p.Current.IsReadOnly
  }
  $p = $null
  if ($el.TryGetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern, [ref]$p)) {
    $row.rangeValue = $p.Current.Value
    $row.rangeMin = $p.Current.Minimum
    $row.rangeMax = $p.Current.Maximum
  }
  try {
    $lvl = $el.GetCurrentPropertyValue([System.Windows.Automation.AutomationElement]::LevelProperty)
    if ($null -ne $lvl -and $lvl -ne 0) { $row.level = $lvl }
  } catch {}
  $script:out.Add([pscustomobject]$row)
}

function Walk($el, $depth) {
  if ($depth -gt $MaxDepth -or $script:count -ge $MaxNodes) { return }
  Emit $el $depth
  $child = $walker.GetFirstChild($el)
  while ($null -ne $child -and $script:count -lt $MaxNodes) {
    Walk $child ($depth + 1)
    $child = $walker.GetNextSibling($child)
  }
}

foreach ($w in $windows) { Walk $w 0 }

# The array wrapper keeps a one-element result a JSON array rather than a bare
# object, and the `[array]` cast is what stops ConvertTo-Json binding the
# generic List to its -InputObject parameter by type.
$rows = [array]$out.ToArray()
ConvertTo-Json -InputObject $rows -Depth 4 -Compress

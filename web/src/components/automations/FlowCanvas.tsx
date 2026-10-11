import { useEffect, useMemo } from "react";
import {
  Background,
  BackgroundVariant,
  Controls,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { nodeTypes } from "@/components/automations/StepNode";
import type { CanvasNode } from "@/components/automations/StepNode";
import { toFlow } from "@/lib/automations";
import type { Draft } from "@/lib/automations";
import { useDarkMode } from "@/lib/theme";

export interface FlowCanvasProps {
  rule: Pick<Draft, "trigger" | "conditions" | "actions">;
  selected?: string | null;
  /** Left out, the flow is only looked at: nothing selects, nothing
      is removed, and a finger on it scrolls the page. */
  onSelect?: (id: string | null) => void;
  onRemove?: (id: string) => void;
}

const FIT = { padding: 0.2, maxZoom: 1 };

/**
 * The rule drawn as a flow, trigger at the top.
 *
 * Nothing on it changes what the rule means: nodes cannot be dragged
 * or joined, because the order is the chain. Adding, removing and
 * reordering happen in the palette and the side panel.
 */
export function FlowCanvas(props: FlowCanvasProps) {
  return (
    <ReactFlowProvider>
      <Canvas {...props} />
    </ReactFlowProvider>
  );
}

function Canvas({ rule, selected, onSelect, onRemove }: FlowCanvasProps) {
  const dark = useDarkMode();
  const { fitView } = useReactFlow();
  const flow = useMemo(() => toFlow(rule), [rule.trigger, rule.conditions, rule.actions]);
  const nodes = useMemo(
    () =>
      flow.nodes.map((node): CanvasNode => ({
        ...node,
        selected: node.id === selected,
        data: {
          ...node.data,
          onRemove: onRemove && node.id !== "trigger" ? () => onRemove(node.id) : undefined,
        },
      })),
    [flow, selected, onRemove],
  );

  // `fitView` on the component fits once, on the first render. A step
  // added or removed changes the picture's height, and the new one has
  // to be fitted again — after React Flow has drawn it.
  const shape = flow.nodes.map((n) => n.id).join(",");
  useEffect(() => {
    const frame = requestAnimationFrame(() => void fitView(FIT));
    return () => cancelAnimationFrame(frame);
  }, [shape, fitView]);

  const editable = onSelect !== undefined;
  return (
    <ReactFlow
      nodes={nodes}
      edges={flow.edges}
      nodeTypes={nodeTypes}
      colorMode={dark ? "dark" : "light"}
      fitView
      fitViewOptions={FIT}
      minZoom={0.3}
      nodesDraggable={false}
      nodesConnectable={false}
      edgesFocusable={false}
      elementsSelectable={editable}
      panOnDrag={editable}
      zoomOnScroll={editable}
      zoomOnPinch={editable}
      zoomOnDoubleClick={false}
      preventScrolling={editable}
      onNodeClick={(_, node) => onSelect?.(node.id)}
      onPaneClick={() => onSelect?.(null)}
      proOptions={{ hideAttribution: true }}
    >
      <Background variant={BackgroundVariant.Dots} gap={20} color="var(--border)" />
      {editable && <Controls showInteractive={false} />}
    </ReactFlow>
  );
}

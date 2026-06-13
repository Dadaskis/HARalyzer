import { Undo2, Redo2, Trash2, Save, Eraser } from "lucide-react";
import { Button } from "@/components/ui/button";

interface EntryEditToolbarProps {
  selectedCount: number;
  canUndo: boolean;
  canRedo: boolean;
  onUndo: () => void;
  onRedo: () => void;
  onDeleteSelected: () => void;
  onDeleteUnselected: () => void;
  onSaveHar: () => void;
}

export function EntryEditToolbar({
  selectedCount,
  canUndo,
  canRedo,
  onUndo,
  onRedo,
  onDeleteSelected,
  onDeleteUnselected,
  onSaveHar,
}: EntryEditToolbarProps) {
  return (
    <div className="flex flex-wrap items-center gap-2 border-b border-primary/10 bg-primary/5 px-4 py-2">
      <span className="text-xs font-medium text-primary">Edit mode</span>
      <Button size="sm" variant="outline" className="h-7 text-xs" onClick={onUndo} disabled={!canUndo}>
        <Undo2 className="h-3.5 w-3.5" />
        Undo
      </Button>
      <Button size="sm" variant="outline" className="h-7 text-xs" onClick={onRedo} disabled={!canRedo}>
        <Redo2 className="h-3.5 w-3.5" />
        Redo
      </Button>
      <Button
        size="sm"
        variant="destructive"
        className="h-7 text-xs"
        onClick={onDeleteSelected}
        disabled={selectedCount === 0}
      >
        <Trash2 className="h-3.5 w-3.5" />
        Delete selected{selectedCount > 0 ? ` (${selectedCount})` : ""}
      </Button>
      <Button size="sm" variant="outline" className="h-7 text-xs" onClick={onDeleteUnselected}>
        <Eraser className="h-3.5 w-3.5" />
        Delete unselected
      </Button>
      <Button size="sm" variant="secondary" className="ml-auto h-7 text-xs" onClick={onSaveHar}>
        <Save className="h-3.5 w-3.5" />
        Save HAR
      </Button>
    </div>
  );
}

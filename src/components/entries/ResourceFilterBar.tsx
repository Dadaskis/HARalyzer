import { RESOURCE_FILTERS, type ResourceFilterId } from "@/lib/entry-filters";
import { cn } from "@/lib/utils";

interface ResourceFilterBarProps {
  value: ResourceFilterId;
  onChange: (value: ResourceFilterId) => void;
  editMode?: boolean;
  allVisibleSelected?: boolean;
  onToggleSelectAllVisible?: () => void;
}

export function ResourceFilterBar({
  value,
  onChange,
  editMode,
  allVisibleSelected,
  onToggleSelectAllVisible,
}: ResourceFilterBarProps) {
  return (
    <div className="flex flex-wrap items-center gap-1 border-b border-primary/10 px-4 py-1.5">
      {editMode && onToggleSelectAllVisible && (
        <label className="mr-2 flex shrink-0 cursor-pointer items-center gap-1.5 rounded border border-primary/20 bg-primary/5 px-2 py-1 text-[11px] text-muted-foreground">
          <input
            type="checkbox"
            checked={allVisibleSelected ?? false}
            onChange={onToggleSelectAllVisible}
            className="h-3 w-3 accent-primary"
          />
          All visible
        </label>
      )}
      {RESOURCE_FILTERS.map((filter) => (
        <button
          key={filter.id}
          type="button"
          onClick={() => onChange(filter.id)}
          className={cn(
            "shrink-0 rounded px-2 py-1 text-[11px] font-medium transition-colors",
            value === filter.id
              ? "bg-primary/20 text-primary"
              : "text-muted-foreground hover:bg-primary/10 hover:text-foreground"
          )}
        >
          {filter.label}
        </button>
      ))}
    </div>
  );
}

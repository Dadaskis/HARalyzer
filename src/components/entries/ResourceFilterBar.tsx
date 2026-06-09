import { RESOURCE_FILTERS, type ResourceFilterId } from "@/lib/entry-filters";
import { cn } from "@/lib/utils";

interface ResourceFilterBarProps {
  value: ResourceFilterId;
  onChange: (value: ResourceFilterId) => void;
}

export function ResourceFilterBar({ value, onChange }: ResourceFilterBarProps) {
  return (
    <div className="flex gap-1 overflow-x-auto border-b border-primary/10 px-4 py-1.5">
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

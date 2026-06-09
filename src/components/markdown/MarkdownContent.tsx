import { memo, useMemo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { cn } from "@/lib/utils";
import { MarkdownCodeBlock } from "@/components/markdown/MarkdownCodeBlock";
import "highlight.js/styles/github-dark-dimmed.css";

interface MarkdownContentProps {
  content: string;
  className?: string;
  /** When false, code blocks may scroll horizontally (e.g. wide log dumps). */
  wrap?: boolean;
}

const markdownPlugins = [remarkGfm];
const rehypePlugins = [rehypeHighlight];

export const MarkdownContent = memo(function MarkdownContent({
  content,
  className,
  wrap = true,
}: MarkdownContentProps) {
  const components = useMemo(
    () => ({
      pre: ({ children }: { children?: React.ReactNode }) => (
        <MarkdownCodeBlock wrap={wrap}>{children}</MarkdownCodeBlock>
      ),
      code: ({
        className: codeClassName,
        children,
        ...props
      }: {
        className?: string;
        children?: React.ReactNode;
      }) => {
        const isBlock = codeClassName?.includes("language-");
        if (isBlock) {
          return (
            <code
              className={cn(
                "font-mono",
                wrap && "[overflow-wrap:anywhere] [word-break:break-word]",
                codeClassName
              )}
              {...props}
            >
              {children}
            </code>
          );
        }
        return (
          <code
            className={cn(
              "rounded bg-muted/80 px-1 py-0.5 font-mono text-[0.85em] text-primary/90",
              wrap && "[overflow-wrap:anywhere] [word-break:break-word]"
            )}
            {...props}
          >
            {children}
          </code>
        );
      },
      a: ({ href, children }: { href?: string; children?: React.ReactNode }) => (
        <a
          href={href}
          target="_blank"
          rel="noreferrer"
          className="break-all text-primary underline-offset-2 hover:underline"
        >
          {children}
        </a>
      ),
      table: ({ children }: { children?: React.ReactNode }) => (
        <div className="my-3 max-w-full overflow-x-auto">
          <table className="w-full min-w-0 border-collapse text-xs">{children}</table>
        </div>
      ),
      ul: ({ children }: { children?: React.ReactNode }) => (
        <ul className="my-2 list-disc space-y-1 break-words pl-5 [overflow-wrap:anywhere]">
          {children}
        </ul>
      ),
      ol: ({ children }: { children?: React.ReactNode }) => (
        <ol className="my-2 list-decimal space-y-1 break-words pl-5 [overflow-wrap:anywhere]">
          {children}
        </ol>
      ),
      li: ({ children }: { children?: React.ReactNode }) => (
        <li className="break-words [overflow-wrap:anywhere]">{children}</li>
      ),
      h1: ({ children }: { children?: React.ReactNode }) => (
        <h1 className="mb-2 mt-4 break-words text-lg font-semibold [overflow-wrap:anywhere]">
          {children}
        </h1>
      ),
      h2: ({ children }: { children?: React.ReactNode }) => (
        <h2 className="mb-2 mt-3 break-words text-base font-semibold [overflow-wrap:anywhere]">
          {children}
        </h2>
      ),
      h3: ({ children }: { children?: React.ReactNode }) => (
        <h3 className="mb-1 mt-2 break-words text-sm font-semibold [overflow-wrap:anywhere]">
          {children}
        </h3>
      ),
      p: ({ children }: { children?: React.ReactNode }) => (
        <p className="my-2 break-words [overflow-wrap:anywhere]">{children}</p>
      ),
      blockquote: ({ children }: { children?: React.ReactNode }) => (
        <blockquote className="my-2 break-words border-l-2 border-primary/40 pl-3 text-muted-foreground [overflow-wrap:anywhere]">
          {children}
        </blockquote>
      ),
    }),
    [wrap]
  );

  return (
    <div
      className={cn(
        "markdown-body text-sm leading-relaxed",
        wrap && "min-w-0 max-w-full [overflow-wrap:anywhere] [word-break:break-word]",
        className
      )}
    >
      <ReactMarkdown
        remarkPlugins={markdownPlugins}
        rehypePlugins={rehypePlugins}
        components={components}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
});

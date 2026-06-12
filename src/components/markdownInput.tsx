type MarkdownInputProps = {
  type?: string | null;
  checked?: boolean | null;
  disabled?: boolean | null;
};

export function renderMarkdownInput({
  type,
  checked,
  disabled,
}: MarkdownInputProps) {
  if (type !== "checkbox") return null;
  return (
    <input
      type="checkbox"
      checked={Boolean(checked)}
      disabled={disabled ?? true}
      readOnly
      className="mr-1.5 align-middle"
    />
  );
}

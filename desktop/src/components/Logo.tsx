import { MessageCircle } from "lucide-react";

export function Logo({ size = 36 }: { size?: number }) {
  return (
    <div
      className="flex items-center justify-center rounded-2xl bg-gradient-to-br from-[#2dd4bf] via-wp-accent to-wp-accent-strong text-white shadow-lg shadow-wp-accent/25"
      style={{ width: size, height: size }}
    >
      <MessageCircle className="h-[55%] w-[55%]" strokeWidth={2.2} />
    </div>
  );
}

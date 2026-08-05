import { MessageCircle } from "lucide-react";

export function Logo({ size = 36 }: { size?: number }) {
  return (
    <div
      className="flex items-center justify-center rounded-2xl bg-gradient-to-br from-violet-500 via-indigo-500 to-cyan-400 text-white shadow-lg shadow-violet-500/25"
      style={{ width: size, height: size }}
    >
      <MessageCircle className="h-[55%] w-[55%]" strokeWidth={2.2} />
    </div>
  );
}

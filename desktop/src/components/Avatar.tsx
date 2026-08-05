import { User } from "lucide-react";

interface AvatarProps {
  name?: string;
  size?: number;
}

export function Avatar({ name, size = 40 }: AvatarProps) {
  return (
    <div
      className="flex shrink-0 select-none items-center justify-center rounded-full bg-gradient-to-br from-wp-accent/85 to-wp-accent-strong text-wp-deep"
      style={{ width: size, height: size }}
    >
      {name ? (
        <span className="font-semibold" style={{ fontSize: size * 0.42 }}>
          {name.charAt(0).toUpperCase()}
        </span>
      ) : (
        <User style={{ width: size * 0.5, height: size * 0.5 }} strokeWidth={2.2} />
      )}
    </div>
  );
}

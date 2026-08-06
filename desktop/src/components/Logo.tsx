import whisperLogo from "../assets/whisper-logo.png";

export function Logo({ size = 36 }: { size?: number }) {
  return (
    <img
      src={whisperLogo}
      alt="Whisper"
      width={size}
      height={size}
      className="select-none"
      draggable={false}
      style={{ width: size, height: size, objectFit: "contain" }}
    />
  );
}

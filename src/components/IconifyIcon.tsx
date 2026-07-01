import { type CSSProperties, type HTMLAttributes } from "react";

export type IconifyIconClassName = string;

export default function IconifyIcon({
  iconClassName,
  className,
  style,
  ...props
}: {
  iconClassName: IconifyIconClassName;
  className?: string;
  style?: CSSProperties;
} & Omit<HTMLAttributes<HTMLSpanElement>, "children">) {
  return (
    <span
      aria-hidden={props["aria-label"] ? undefined : true}
      {...props}
      className={[iconClassName, className ?? ""].filter(Boolean).join(" ")}
      style={style}
    />
  );
}

type IconifyIconComponentProps = {
  className?: string;
  style?: CSSProperties;
};

export function HashIcon({ className, style }: IconifyIconComponentProps) {
  return <IconifyIcon iconClassName="icon-[mynaui--hash]" className={className} style={style} />;
}

export function BitcoinHashesOutlineIcon({ className, style }: IconifyIconComponentProps) {
  return <IconifyIcon iconClassName="icon-[bitcoin-icons--hashes-filled]" className={className} style={style} />;
}

export function Robot3LineIcon({ className, style }: IconifyIconComponentProps) {
  return <IconifyIcon iconClassName="icon-[ri--robot-3-line]" className={className} style={style} />;
}

export function PeopleTeam24RegularIcon({ className, style }: IconifyIconComponentProps) {
  return <IconifyIcon iconClassName="icon-[fluent--people-team-24-regular]" className={className} style={style} />;
}

export function Robot3FillIcon({ className, style }: IconifyIconComponentProps) {
  return <IconifyIcon iconClassName="icon-[ri--robot-3-fill]" className={className} style={style} />;
}

export function AiGenerate2Icon({ className, style }: IconifyIconComponentProps) {
  return <IconifyIcon iconClassName="icon-[ri--ai-generate-2]" className={className} style={style} />;
}

export function ChannelShare24RegularIcon({ className, style }: IconifyIconComponentProps) {
  return <IconifyIcon iconClassName="icon-[fluent--channel-share-24-regular]" className={className} style={style} />;
}

export function TelegramLogoIcon({ className, style }: IconifyIconComponentProps) {
  return <IconifyIcon iconClassName="icon-[streamline-logos--telegram-logo-2-solid]" className={className} style={style} />;
}

export function DiscordLogoIcon({ className, style }: IconifyIconComponentProps) {
  return <IconifyIcon iconClassName="icon-[streamline-logos--discord-logo-2-solid]" className={className} style={style} />;
}

export function LarkLogoIcon({ className, style }: IconifyIconComponentProps) {
  return <IconifyIcon iconClassName="icon-[icon-park-outline--new-lark]" className={className} style={style} />;
}

export function WechatLogoIcon({ className, style }: IconifyIconComponentProps) {
  return <IconifyIcon iconClassName="icon-[streamline-logos--wechat-logo-solid]" className={className} style={style} />;
}

export function TokenOutlineIcon({ className, style }: IconifyIconComponentProps) {
  return <IconifyIcon iconClassName="icon-[material-symbols--token-outline]" className={className} style={style} />;
}

export function QrCodeIcon({ className, style }: IconifyIconComponentProps) {
  return <IconifyIcon iconClassName="icon-[material-symbols--qr-code]" className={className} style={style} />;
}

export function ScreenshotRegionIcon({ className, style }: IconifyIconComponentProps) {
  return <IconifyIcon iconClassName="icon-[material-symbols--screenshot-region]" className={className} style={style} />;
}

export function HashtagChatLinearIcon({ className, style }: IconifyIconComponentProps) {
  return <IconifyIcon iconClassName="icon-[solar--hashtag-chat-linear]" className={className} style={style} />;
}

export function CodeXmlIcon({ className, style }: IconifyIconComponentProps) {
  return <IconifyIcon iconClassName="icon-[material-symbols--code-xml]" className={className} style={style} />;
}

export function PiIcon({ className, style }: IconifyIconComponentProps) {
  return <IconifyIcon iconClassName="icon-[simple-icons--pi]" className={["scale-90", className ?? ""].filter(Boolean).join(" ")} style={style} />;
}

export function OpencodeIcon({ className, style }: IconifyIconComponentProps) {
  return <IconifyIcon iconClassName="icon-[simple-icons--opencode]" className={className} style={style} />;
}

import {
  BookOpen,
  ClipboardCheck,
  Clock3,
  FolderClosed,
  Globe,
  ImagePlus,
  Images,
  ListTodo,
  Pencil,
  ScanText,
  Search,
  SquareTerminal,
  UsersRound,
  Wifi,
  Wrench,
  type LucideIcon,
} from "lucide-react";

/** Visual vocabulary only: event classification and summary priority belong to their hosts. */
export const agentActivityIcons = {
  tool: Wrench,
  edit: Pencil,
  search: Search,
  read: BookOpen,
  command: SquareTerminal,
  listFiles: FolderClosed,
  web: Globe,
  image: Images,
  imageGeneration: ImagePlus,
  collaboration: UsersRound,
  reconnect: Wifi,
  compaction: ScanText,
  wait: Clock3,
  plan: ListTodo,
  review: ClipboardCheck,
} as const satisfies Readonly<Record<string, LucideIcon>>;

export type AgentActivityIconKind = keyof typeof agentActivityIcons;

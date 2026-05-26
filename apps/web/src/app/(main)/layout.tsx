import { Header } from "@/components/Header";

export default function MainLayout({ children }: { children: React.ReactNode }) {
  return (
    <>
      <Header />
      <main className="flex flex-col min-h-[calc(100vh-69px)]">
        {children}
      </main>
    </>
  );
}

import { RegistryClient } from "../components/RegistryClient";
import { skills } from "../lib/catalog";

export default function Home() {
  return <RegistryClient skills={skills} />;
}

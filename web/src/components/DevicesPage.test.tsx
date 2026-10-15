import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { BrokerStatus, DevicesPage } from "@/components/DevicesPage";
import type { DevicesPageProps } from "@/components/DevicesPage";

function setup(extra: Partial<DevicesPageProps> = {}) {
  const onChange = vi.fn();
  render(
    <DevicesPage
      mqtt={{ host: "10.0.0.2" }}
      defaults={{ port: 1883, z2m_prefix: "zigbee2mqtt", client_id: "niles" }}
      username={{ key: "mqtt.username", label: "Zigbee2MQTT broker username", source: "unset" }}
      password={{ key: "mqtt.password", label: "Zigbee2MQTT broker password", source: "unset" }}
      writable
      devices={12}
      gaps={[]}
      onChange={onChange}
      onSecretsChanged={vi.fn()}
      {...extra}
    />,
  );
  return { onChange };
}

describe("DevicesPage", () => {
  it("sets the broker up, login and all", () => {
    const { onChange } = setup();
    const host = screen.getByLabelText("Broker address");
    expect(host).toHaveValue("10.0.0.2");
    expect(screen.getByLabelText("Port")).toHaveAttribute("placeholder", "1883");
    expect(screen.getByLabelText("Zigbee2MQTT broker password")).toBeInTheDocument();
    expect(screen.getByText(/take effect when Niles restarts/)).toBeInTheDocument();

    fireEvent.change(host, { target: { value: "mosquitto.local" } });
    fireEvent.blur(host);
    expect(onChange).toHaveBeenCalledWith([{ path: "mqtt.host", value: "mosquitto.local" }]);
  });

  it("leads with what is missing on a new install", () => {
    setup({
      mqtt: {},
      gaps: [
        {
          path: "mqtt.host",
          severity: "blocking",
          consequence: "Niles can't reach any lights yet: it has no broker to talk to.",
        },
      ],
    });
    expect(screen.getByText(/can't reach any lights yet/)).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: /missing something it needs/ })).toBeInTheDocument();
  });
});

describe("BrokerStatus", () => {
  const text = (host?: string, devices?: number) =>
    render(<BrokerStatus host={host} devices={devices} />).container.textContent;

  it("says there is no broker before one is set", () => {
    expect(text(undefined, 0)).toMatch(/No broker yet/);
  });

  it("says what has come through it", () => {
    expect(text("10.0.0.2", 12)).toBe("Set to 10.0.0.2. Niles knows 12 devices.");
    expect(text("10.0.0.2", 1)).toBe("Set to 10.0.0.2. Niles knows 1 device.");
  });

  it("says when nothing has, which is the sign something is wrong", () => {
    expect(text("10.0.0.2", 0)).toMatch(/nothing has come through it yet/);
  });
});

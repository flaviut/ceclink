import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location(
    "physical_address", Path(__file__).resolve().parents[1] / "nix/physical-address.py"
)
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)


class PhysicalAddressTests(unittest.TestCase):
    def test_status_format_and_settling(self):
        filt = helper.AddressFilter()
        self.assertEqual(helper.parse_status(b"status_version=1\tphysical_address=4096\n"), 0x1000)
        self.assertFalse(filt.settled(0x1000, 0.0))
        self.assertTrue(filt.settled(0x1000, 1.0))
        self.assertFalse(filt.settled(0xFFFF, 1.1))
        self.assertFalse(filt.settled(0xFFFF, 2.1))
        self.assertTrue(filt.settled(0xFFFF, 3.1))
        self.assertFalse(filt.settled(0x2000, 3.2))
        self.assertTrue(filt.settled(0x2000, 4.2))
        for line in (
            b"version=1\tphysical_address=4096\n",
            b"status_version=2\tphysical_address=4096\n",
            b"status_version=1\tphysical_address=4097\n",
            b"status_version=1\tphysical_address=65536\n",
            b"status_version=1\n",
            b"\xff\n",
        ):
            self.assertIsNone(helper.parse_status(line), line)

    def test_cec_matching_follows_common_usb_parent(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "class/tty").mkdir(parents=True)
            (root / "dev/char").mkdir(parents=True)
            (root / "devnodes").mkdir()
            syspaths = {}
            for usb_name, tty_name, cec_name in (("1-2", "ttyACM1", "cec7"), ("1-3", "ttyACM3", "cec0")):
                usb = root / "devices" / usb_name
                tty = usb / "debug" / tty_name
                cec = usb / "control/tty/serio" / cec_name
                tty.mkdir(parents=True)
                cec.mkdir(parents=True)
                (usb / "idVendor").write_text("2548")
                (usb / "idProduct").write_text("1002")
                (root / "class/tty" / tty_name).symlink_to(tty)
                node = root / "devnodes" / cec_name
                node.touch()
                syspaths[node] = cec
            (root / "devnodes/ceclink-status").symlink_to(root / "devnodes/cec7")
            with patch.object(helper, "cec_sysfs_path", side_effect=lambda node, _sysfs: syspaths[node]):
                self.assertEqual(helper.matching_cec("ttyACM1", root, root / "devnodes"), root / "devnodes/cec7")
                self.assertEqual(helper.matching_cec("ttyACM3", root, root / "devnodes"), root / "devnodes/cec0")

    def test_retries_after_driver_reset_with_same_status(self):
        with tempfile.NamedTemporaryFile() as temporary:
            current = [0xFFFF]
            writes = []

            def ioctl(_port, request, value, *_args):
                if request == helper.CEC_ADAP_G_PHYS_ADDR:
                    value[0] = current[0]
                else:
                    self.assertEqual(request, helper.CEC_ADAP_S_PHYS_ADDR)
                    writes.append(value[0])
                    current[0] = value[0]

            with patch.object(helper.fcntl, "ioctl", side_effect=ioctl):
                device = Path(temporary.name)
                self.assertTrue(helper.apply_address(device, 0x1000))
                self.assertFalse(helper.apply_address(device, 0x1000))
                current[0] = 0xFFFF
                self.assertTrue(helper.apply_address(device, 0x1000))
                self.assertTrue(helper.apply_address(device, 0xFFFF))
            self.assertEqual(writes, [0x1000, 0x1000, 0xFFFF])

    def test_ddc_buses_cover_all_hdmi_connectors(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            drm = root / "class/drm"
            drm.mkdir(parents=True)
            for name, status, bus in (
                ("card1-HDMI-A-1", "connected", 5),
                ("card1-HDMI-A-2", "disconnected", 6),
                ("card2-HDMI-A-1", "connected", 9),
                ("card2-DP-1", "connected", 10),
            ):
                connector = drm / name
                connector.mkdir()
                (connector / "status").write_text(status)
                i2c = root / f"i2c-{bus}"
                i2c.mkdir()
                (connector / "ddc").symlink_to(i2c)
            self.assertEqual(helper.ddc_buses(root), [5, 6, 9])

    def test_recovery_reads_every_hdmi_bus_and_bounds_retries(self):
        recovery = helper.EdidRecovery()
        with patch.object(helper, "ddc_buses", return_value=[5, 9]), patch.object(
            helper.subprocess, "run", return_value=subprocess.CompletedProcess([], 1)
        ) as run, patch.object(helper.time, "monotonic", side_effect=[1.0, 7.0, 13.0]):
            self.assertTrue(recovery.observe(0xFFFF, 0.0))
            self.assertTrue(recovery.observe(0xFFFF, 2.0))
            self.assertTrue(recovery.observe(0xFFFF, 6.0))
            self.assertTrue(recovery.observe(0xFFFF, 12.0))
            self.assertTrue(recovery.observe(0xFFFF, 15.0))
            self.assertFalse(recovery.observe(0xFFFF, 18.0))
            self.assertEqual(run.call_count, 6)
            self.assertEqual(
                [call.args[0][1] for call in run.call_args_list],
                ["--bus=5", "--bus=9"] * 3,
            )
            for call in run.call_args_list:
                self.assertIn("--edid-read-size=256", call.args[0])
                self.assertIn("--disable-try-get-edid-from-sysfs", call.args[0])
            self.assertFalse(recovery.observe(0x3000, 19.0))
            self.assertEqual(recovery.attempts, 0)


if __name__ == "__main__":
    unittest.main()

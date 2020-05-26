extern crate krill;

use std::str::FromStr;

use krill::commons::api::{
    Handle, ParentCaReq, ResourceSet, RoaDefinition, RoaDefinitionUpdates,
};
use krill::daemon::ca;
use krill::daemon::ca::ta_handle;
use krill::daemon::ca::{RoaPrefixGroupingStrategy};
use krill::daemon::test::*;

#[test]
/// Test the CAs can issue and publish ROAs for their resources, and that
/// ROAs get updated and published properly when resources change, as well
/// as during and after key rolls.
fn ca_roa_per_asn_grouping() {
    test_with_krill_server(|_d| {
        ca::set_roa_prefix_grouping_strategy(RoaPrefixGroupingStrategy::RoaPerAsn);
        let ta_handle = ta_handle();
        let child = Handle::from_str_unsafe("child");
        let child_resources = ResourceSet::from_strs("", "10.0.0.0/16", "2001:DB8::/32").unwrap();

        init_child_with_embedded_repo(&child);

        // Set up under parent  ----------------------------------------------------------------
        {
            let parent = {
                let parent_contact = add_child_to_ta_embedded(&child, child_resources.clone());
                ParentCaReq::new(ta_handle.clone(), parent_contact)
            };
            add_parent_to_ca(&child, parent);
            wait_for_current_resources(&child, &child_resources);
        }

        // Add some Route Authorizations
        let route_1 = RoaDefinition::from_str("10.0.0.0/24 => 64496").unwrap();
        let route_2 = RoaDefinition::from_str("2001:DB8::/32-48 => 64496").unwrap();
        let route_3 = RoaDefinition::from_str("192.168.0.0/24 => 64496").unwrap();
        let route_4 = RoaDefinition::from_str("192.168.1.0/24 => 0").unwrap();

        let crl_file = ".crl";
        let mft_file = ".mft";
        let roa_file = ".roa";

        wait_for_published_objects(&child, &[crl_file, mft_file]);
        assert_eq!(count_roa_files(&child), 0);

        let mut updates = RoaDefinitionUpdates::empty();
        updates.add(route_1);
        updates.add(route_2);
        ca_route_authorizations_update(&child, updates);
        wait_for_published_objects(&child, &[crl_file, mft_file, roa_file]);
        assert_eq!(count_roa_files(&child), 1);
        assert!(roas_contain_route(&child, &route_1));
        assert!(roas_contain_route(&child, &route_2));

        // Remove a Route Authorization
        let mut updates = RoaDefinitionUpdates::empty();
        updates.remove(route_1);
        let previous_published_objects = get_published_objects(&child);
        ca_route_authorizations_update(&child, updates);
        wait_for_updated_published_objects(&child, previous_published_objects);
        wait_for_published_objects(&child, &[crl_file, mft_file, roa_file]);
        assert_eq!(count_roa_files(&child), 1);
        assert_eq!(roas_contain_route(&child, &route_1), false);
        assert!(roas_contain_route(&child, &route_2));

        // Refuse authorization for prefix not held by CA
        let mut updates = RoaDefinitionUpdates::empty();
        updates.add(route_3);
        ca_route_authorizations_update_expect_error(&child, updates);
        assert_eq!(roas_contain_route(&child, &route_3), false);

        // Shrink resources and see that ROA is removed
        let child_resources = ResourceSet::from_strs("", "192.168.0.0/16", "").unwrap();
        update_child(&ta_handle, &child, &child_resources);
        wait_for_published_objects(&child, &[crl_file, mft_file]);
        assert_eq!(count_roa_files(&child), 0);
        assert_eq!(roas_contain_route(&child, &route_1), false);
        assert_eq!(roas_contain_route(&child, &route_2), false);

        // Now route3 can be added
        let mut updates = RoaDefinitionUpdates::empty();
        updates.add(route_3);
        ca_route_authorizations_update(&child, updates);
        wait_for_published_objects(&child, &[crl_file, mft_file, roa_file]);
        assert_eq!(count_roa_files(&child), 1);
        assert!(roas_contain_route(&child, &route_3));

        // Adds route4 with different ASN
        let mut updates = RoaDefinitionUpdates::empty();
        updates.add(route_4);
        ca_route_authorizations_update(&child, updates);
        wait_for_published_objects(&child, &[crl_file, mft_file, roa_file, roa_file]);
        assert_eq!(count_roa_files(&child), 2);
        assert!(roas_contain_route(&child, &route_3));
        assert!(roas_contain_route(&child, &route_4));
    });
}

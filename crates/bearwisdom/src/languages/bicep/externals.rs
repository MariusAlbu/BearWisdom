/// Azure resource type provider namespaces always classified as external.
/// These appear in resource declarations as `'Microsoft.Compute/virtualMachines@...'`.
pub(crate) const EXTERNALS: &[&str] = &[
    // ── Compute ───────────────────────────────────────────────────────────────
    "Microsoft.Compute/virtualMachines",
    "Microsoft.Compute/virtualMachineScaleSets",
    "Microsoft.Compute/disks",
    "Microsoft.Compute/availabilitySets",
    "Microsoft.Compute/images",
    "Microsoft.Compute/snapshots",
    // ── Networking ────────────────────────────────────────────────────────────
    "Microsoft.Network/virtualNetworks",
    "Microsoft.Network/networkInterfaces",
    "Microsoft.Network/publicIPAddresses",
    "Microsoft.Network/networkSecurityGroups",
    "Microsoft.Network/loadBalancers",
    "Microsoft.Network/applicationGateways",
    "Microsoft.Network/routeTables",
    "Microsoft.Network/privateDnsZones",
    "Microsoft.Network/dnsZones",
    "Microsoft.Network/firewalls",
    "Microsoft.Network/bastionHosts",
    "Microsoft.Network/virtualNetworkGateways",
    "Microsoft.Network/connections",
    "Microsoft.Network/privateEndpoints",
    "Microsoft.Network/privateLinkServices",
    // ── Storage ───────────────────────────────────────────────────────────────
    "Microsoft.Storage/storageAccounts",
    "Microsoft.Storage/storageAccounts/blobServices",
    "Microsoft.Storage/storageAccounts/fileServices",
    // ── Web / App Service ─────────────────────────────────────────────────────
    "Microsoft.Web/sites",
    "Microsoft.Web/serverfarms",
    "Microsoft.Web/staticSites",
    "Microsoft.Web/certificates",
    // ── Key Vault ─────────────────────────────────────────────────────────────
    "Microsoft.KeyVault/vaults",
    "Microsoft.KeyVault/vaults/secrets",
    "Microsoft.KeyVault/vaults/keys",
    // ── Containers ───────────────────────────────────────────────────────────
    "Microsoft.ContainerService/managedClusters",
    "Microsoft.ContainerRegistry/registries",
    "Microsoft.ContainerInstance/containerGroups",
    // ── Databases / SQL ──────────────────────────────────────────────────────
    "Microsoft.Sql/servers",
    "Microsoft.Sql/servers/databases",
    "Microsoft.Sql/servers/firewallRules",
    "Microsoft.Sql/servers/elasticPools",
    "Microsoft.DocumentDB/databaseAccounts",
    "Microsoft.DBforPostgreSQL/flexibleServers",
    "Microsoft.DBforMySQL/flexibleServers",
    // ── Identity ─────────────────────────────────────────────────────────────
    "Microsoft.ManagedIdentity/userAssignedIdentities",
    "Microsoft.Authorization/roleAssignments",
    "Microsoft.Authorization/roleDefinitions",
    // ── Monitoring / Insights ────────────────────────────────────────────────
    "Microsoft.OperationalInsights/workspaces",
    "Microsoft.Insights/components",
    "Microsoft.Insights/diagnosticSettings",
    "Microsoft.Insights/metricAlerts",
    "Microsoft.Insights/activityLogAlerts",
    "Microsoft.Insights/actionGroups",
    // ── Service Bus / Event / Messaging ─────────────────────────────────────
    "Microsoft.ServiceBus/namespaces",
    "Microsoft.ServiceBus/namespaces/queues",
    "Microsoft.ServiceBus/namespaces/topics",
    "Microsoft.EventHub/namespaces",
    "Microsoft.EventHub/namespaces/eventhubs",
    "Microsoft.EventGrid/topics",
    "Microsoft.EventGrid/eventSubscriptions",
    // ── Cognitive / AI ───────────────────────────────────────────────────────
    "Microsoft.CognitiveServices/accounts",
    "Microsoft.MachineLearningServices/workspaces",
    // ── CDN / Front Door ─────────────────────────────────────────────────────
    "Microsoft.Cdn/profiles",
    "Microsoft.Cdn/profiles/endpoints",
    "Microsoft.Network/frontDoors",
    // ── Resources ────────────────────────────────────────────────────────────
    "Microsoft.Resources/deployments",
    "Microsoft.Resources/resourceGroups",
    "Microsoft.Resources/deploymentScripts",
    // ── App Configuration / SignalR ───────────────────────────────────────────
    "Microsoft.AppConfiguration/configurationStores",
    "Microsoft.SignalRService/signalR",
    // ── Search ───────────────────────────────────────────────────────────────
    "Microsoft.Search/searchServices",
];

